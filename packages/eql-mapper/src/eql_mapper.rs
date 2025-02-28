use std::{cell::RefCell, ops::ControlFlow, rc::Rc};

use sqltk::{convert_control_flow, Break, Semantic, Visitable, Visitor};

use crate::{
    inference::{Type, TypeError, TypeInferencer},
    Scope, ScopeError,
};

use super::importer::{ImportError, Importer};

#[derive(Debug)]
pub struct EqlMapper {
    scope_tracker: Rc<RefCell<Scope>>,
    importer: Rc<RefCell<Importer>>,
    inferencer: Rc<RefCell<TypeInferencer>>,
}

impl EqlMapper {
    pub fn new(
        scope_tracker: Rc<RefCell<Scope>>,
        importer: Rc<RefCell<Importer>>,
        inferencer: Rc<RefCell<TypeInferencer>>,
    ) -> Self {
        Self {
            scope_tracker,
            importer,
            inferencer,
        }
    }

    pub fn get_type<N: Semantic>(&self, node: &N) -> Rc<RefCell<Type>> {
        self.inferencer.borrow().get_type(node)
    }
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EqlMapperError {
    #[error(transparent)]
    Scope(#[from] ScopeError),

    #[error(transparent)]
    Import(#[from] ImportError),

    #[error(transparent)]
    Type(#[from] TypeError),
}

impl<'ast> Visitor<'ast> for EqlMapper {
    type Error = EqlMapperError;

    fn enter<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        convert_control_flow(self.scope_tracker.borrow_mut().enter(node))?;
        convert_control_flow(self.importer.borrow_mut().enter(node))?;
        convert_control_flow(self.inferencer.borrow_mut().enter(node))?;

        ControlFlow::Continue(())
    }

    fn exit<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        convert_control_flow(self.inferencer.borrow_mut().exit(node))?;
        convert_control_flow(self.importer.borrow_mut().exit(node))?;
        convert_control_flow(self.scope_tracker.borrow_mut().exit(node))?;

        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;

    use std::{cell::RefCell, ops::ControlFlow, rc::Rc, sync::Arc};

    use sqlparser::{
        ast::{Ident, Statement},
        dialect::PostgreSqlDialect,
        parser::Parser,
    };
    use sqltk::{Break, Visitable};

    use crate::{
        eql_mapper::EqlMapper,
        importer::Importer,
        inference::TypeRegistry,
        inference::{Type, Unifier},
        make_schema, Schema, Scope, TypeInferencer,
    };

    use super::EqlMapperError;

    fn parse(statement: &'static str) -> Statement {
        Parser::parse_sql(&PostgreSqlDialect {}, statement).unwrap()[0].clone()
    }

    fn id(ident: &str) -> Ident {
        Ident::from(ident)
    }

    fn check(
        schema: Arc<Schema>,
        statement: &Statement,
    ) -> Result<EqlMapper, (EqlMapperError, EqlMapper)> {
        let scope = Rc::new(RefCell::new(Scope::new()));
        let reg = Rc::new(RefCell::new(TypeRegistry::new()));
        let unifier = RefCell::new(Unifier::new());
        let importer = Importer::new(reg.clone(), schema.clone(), scope.clone());

        let inferencer = Rc::new(RefCell::new(TypeInferencer::new(
            schema.clone(),
            scope.clone(),
            reg.clone(),
            unifier,
        )));

        let mut visitor = EqlMapper::new(scope.clone(), importer.clone(), inferencer.clone());

        let result = statement.accept(&mut visitor);

        match result {
            ControlFlow::Continue(()) => Ok(visitor),
            ControlFlow::Break(Break::Err(err)) => Err((err, visitor)),
            ControlFlow::Break(_) => Ok(visitor),
        }
    }

    #[test]
    fn basic() {
        let schema = Arc::new(make_schema! {
            tables: {
                users: {
                    id (PK),
                    email,
                    first_name,
                }
            }
        });

        let statement = parse("select email from users");

        assert!(check(schema, &statement).is_ok());
    }

    #[test]
    fn select_columns_from_multiple_tables() {
        let schema = Arc::new(make_schema! {
            tables: {
                users: {
                    id (PK),
                    email (ENCRYPTED),
                    first_name,
                }
                todo_lists: {
                    id (PK),
                    name,
                    owner_id,
                    created_at,
                    updated_at,
                }
            }
        });

        let statement = parse(
            r#"
            select
                u.email
            from
                users as u
            inner
                join todo_lists as tl on tl.owner_id = u.id
            ;
            "#,
        );

        let sm = check(schema, &statement).unwrap();

        assert_eq!(
            sm.get_type(&statement),
            Type::projection(&[(Type::encrypted_scalar("users", "email"), Some(id("email"))),])
        )
    }

    #[test]
    fn select_columns_from_subquery() {
        let schema = Arc::new(make_schema! {
            tables: {
                users: {
                    id,
                    email,
                    first_name,
                }
                todo_lists: {
                    id,
                    name,
                    owner_id,
                    created_at,
                    updated_at,
                }
                todo_list_items: {
                    id,
                    description (ENCRYPTED),
                    owner_id,
                    created_at,
                    updated_at,
                }
            }
        });

        let statement = parse(
            r#"
                select
                    u.id as user_id,
                    tli.id as todo_list_item_id,
                    tli.description as todo_list_item_description
                from
                    users as u
                inner join (
                    select
                        id,
                        owner_id,
                        description
                    from
                        todo_list_items
                ) as tli on tli.owner_id = u.id;
            "#,
        );

        match check(schema, &statement) {
            Ok(sm) => {
                assert_eq!(
                    Type::projection(&[
                        (Type::native_scalar("users", "id"), Some(id("user_id"))),
                        (
                            Type::native_scalar("todo_list_items", "id"),
                            Some(id("todo_list_item_id"))
                        ),
                        (
                            Type::encrypted_scalar("todo_list_items", "description"),
                            Some(id("todo_list_item_description"))
                        ),
                    ]),
                    sm.get_type(&statement)
                );
            }
            Err((err, sm)) => {
                sm.inferencer.borrow().dump_registry(&statement);
                panic!("{err}");
            }
        }
    }

    #[test]
    fn wildcard_expansion() {
        let schema = Arc::new(make_schema! {
            tables: {
                users: {
                    id,
                    email,
                }
                todo_lists: {
                    id,
                    owner_id,
                }
            }
        });

        let statement = parse(
            r#"
                select
                    u.*,
                    tl.*
                from
                    users as u
                inner join todo_lists as tl on tl.owner_id = u.id
            "#,
        );

        match check(schema, &statement) {
            Ok(sm) => {
                sm.inferencer.borrow().dump_registry(&statement);
                assert_eq!(
                    Type::projection(&[
                        (Type::native_scalar("users", "id"), None),
                        (Type::native_scalar("users", "email"), None),
                        (Type::native_scalar("todo_lists", "id"), None),
                        (Type::native_scalar("todo_lists", "owner_id"), None),
                    ]),
                    sm.get_type(&statement)
                );
            }
            Err((err, sm)) => {
                sm.inferencer.borrow().dump_registry(&statement);
                panic!("{err}");
            }
        }
    }
}
