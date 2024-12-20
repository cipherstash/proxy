use std::{
    cell::RefCell, collections::HashMap, marker::PhantomData, ops::ControlFlow, rc::Rc, sync::Arc,
};

use sqlparser::ast::{self as ast, Statement};
use sqltk::{convert_control_flow, Break, Transform, Transformable, Visitable, Visitor};

use crate::{
    inference::{unifier, TypeError, TypeInferencer},
    unifier::{EqlValue, Unifier},
    Dep, DepMut, NodeKey, Projection, ProjectionColumn, Schema, ScopeError, ScopeTracker,
    TypeRegistry, Value,
};

use super::importer::{ImportError, Importer};

/// Validates that a SQL statement is well-typed with respect to a database schema that contains zero or more columns with
/// EQL types.
///
/// Specifically, an `Ok` result implies:
///
/// - all referenced tables and columns exist in the schema
/// - concrete types have been inferred for all literals and placeholder expressions
/// - all operators and functions used with literals destined to be transformed to EQL types are semantically valid for
///   that EQL type
///
/// A successful type check will return a [`TypedStatement`] which can be interrogated to discover the required params
/// and their types, the types and plaintext values of all literals, and an optional projection type (the optionality
/// depending on the specific statement).
///
/// Invoking [`TypedStatement::transform`] will return an updated [`Statement`] where all plaintext literals have been
/// replaced with their EQL (encrypted) equivalent and specific SQL operators and functions will have been rewritten to
/// invoke the EQL equivalents.
///
/// An [`EqlMapperError`] is returned if type checking fails.
pub fn type_check<'ast>(
    schema: impl Into<Arc<Schema>>,
    statement: &'ast Statement,
) -> Result<TypedStatement<'ast>, EqlMapperError> {
    let mut mapper = EqlMapper::<'ast>::new_from_schema(schema);
    match statement.accept(&mut mapper) {
        ControlFlow::Continue(()) => {
            // Ensures that there are no unresolved types.
            if let Err(err) = mapper.inferencer.borrow().try_resolve_all_types() {
                #[cfg(test)]
                mapper.inferencer.borrow().dump_registry(statement);

                Err(err)?
            };

            Ok(TypedStatement {
                statement,
                statement_type: mapper.statement_type(statement)?,
                params: mapper.param_types()?,
                literals: mapper.literal_types()?,
            })
        }
        ControlFlow::Break(Break::Err(err)) => {
            #[cfg(test)]
            {
                mapper.inferencer.borrow().dump_registry(statement);
            }

            Err(err)
        }
        ControlFlow::Break(_) => Err(EqlMapperError::InternalError(String::from(
            "unexpected Break value in type_check",
        ))),
    }
}

/// Returns whether the [`Statement`] requires type-checking to be performed.
///
/// Statements that do not require type-checking are presumed to be safe to transmit to the database unmodified.
///
/// This function returns `true` for `MERGE` and `PREPARE` statements even though support for those is not yet
/// implemented in the mapper. Type checking will fail on those statements.
///
/// It is acceptable for `MERGE` because it is rarely used, but when it is used we want a type check to fail.
///
/// It is acceptable for `PREPARE` because we believe that most ORMs do not make direct use of it.
///
/// In any case, support for those statements is coming soon!
pub fn requires_type_check(statement: &Statement) -> bool {
    match statement {
        Statement::Query(_)
        | Statement::Insert(_)
        | Statement::Update { .. }
        | Statement::Delete(_)
        | Statement::Merge { .. }
        | Statement::Prepare { .. } => true, // not
        _ => false,
    }
}

/// The result returned from a successful call to [`type_check`].
#[derive(Debug)]
pub struct TypedStatement<'ast> {
    /// The SQL statement which was type-checked against the schema.
    pub statement: &'ast Statement,

    /// The SQL statement which was type-checked against the schema.
    pub statement_type: Option<Projection>,

    /// The types of all params discovered from [`Value::Placeholder`] nodes in the SQL statement.
    pub params: Vec<Value>,

    /// The types and values of all literals from the SQL statement.
    pub literals: Vec<(EqlValue, &'ast ast::Expr)>,
}

/// The error type returned by various functions in the `eql_mapper` crate.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum EqlMapperError {
    #[error("Error during SQL transformation: {}", _0)]
    Transform(String),

    #[error("Internal error: {}", _0)]
    InternalError(String),

    #[error("Unsupported value variant: {}", _0)]
    UnsupportedValueVariant(String),

    /// A lexical scope error
    #[error(transparent)]
    Scope(#[from] ScopeError),

    /// An error when attempting to import a table or table-column from the database schema
    #[error(transparent)]
    Import(#[from] ImportError),

    /// A type error encountered during type checking
    #[error(transparent)]
    Type(#[from] TypeError),
}

/// `EqlMapper` can safely convert a SQL statement into an equivalent statement where all of the plaintext literals have
/// been converted to EQL payloads containing the encrypted literal and/or encrypted representations of those literals.
#[derive(Debug)]
struct EqlMapper<'ast> {
    scope_tracker: Rc<RefCell<ScopeTracker<'ast>>>,
    importer: Rc<RefCell<Importer<'ast>>>,
    inferencer: Rc<RefCell<TypeInferencer<'ast>>>,
    registry: Rc<RefCell<TypeRegistry<'ast>>>,
    _ast: PhantomData<&'ast ()>,
}

impl<'ast> EqlMapper<'ast> {
    /// Build an `EqlMapper`, initialising all the other visitor implementations that it depends on.
    fn new_from_schema(schema: impl Into<Arc<Schema>>) -> Self {
        let schema = Dep::from(schema.into());
        let scope_tracker = DepMut::new(ScopeTracker::new());
        let registry = DepMut::new(TypeRegistry::new());
        let importer = DepMut::new(Importer::new(&schema, &registry, &scope_tracker));
        let unifier = DepMut::new(Unifier::new(&registry));
        let inferencer = DepMut::new(TypeInferencer::new(
            &schema,
            &scope_tracker,
            &registry,
            &unifier,
        ));

        Self {
            scope_tracker: scope_tracker.into(),
            importer: importer.into(),
            inferencer: inferencer.into(),
            registry: registry.into(),
            _ast: PhantomData,
        }
    }

    /// Asks the [`TypeInferencer`] for a hashmap of node types.
    fn statement_type(
        &self,
        statement: &'ast Statement,
    ) -> Result<Option<Projection>, EqlMapperError> {
        let node_types = self.inferencer.borrow().node_types()?;

        match node_types.get(&NodeKey::new(statement)) {
            Some(ty) => match ty {
                unifier::Type::Constructor(unifier::Constructor::Projection(
                    unifier::Projection::WithColumns(unifier::ProjectionColumns(cols)),
                )) => Ok(Some(Projection::WithColumns(
                    cols.iter()
                        .map(|col| match &col.ty {
                            unifier::Type::Constructor(unifier::Constructor::Value(value)) => {
                                Ok(ProjectionColumn {
                                    ty: value.try_into()?,
                                    alias: col.alias.clone(),
                                })
                            }
                            ty => Err(EqlMapperError::InternalError(format!(
                                "unexpected type {} in projection column",
                                ty
                            ))),
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                ))),
                unifier::Type::Constructor(unifier::Constructor::Projection(
                    unifier::Projection::Empty,
                )) => Ok(Some(Projection::Empty)),
                _ => Err(EqlMapperError::InternalError(String::from(
                    "resolved type for statement was not a resolved projection or empty",
                ))),
            },
            None => Err(EqlMapperError::InternalError(String::from(
                "could not resolve type for statement",
            ))),
        }
    }

    /// Asks the [`TypeInferencer`] for a hashmap of parameter types.
    fn param_types(&self) -> Result<Vec<Value>, EqlMapperError> {
        let param_types = self.inferencer.borrow().param_types()?;

        let mut param_types: Vec<(i32, Value)> = param_types
            .iter()
            .map(|(param, ty)| {
                Value::try_from(ty).and_then(|ty| {
                    param.parse().map(|idx| (idx, ty)).map_err(|_| {
                        EqlMapperError::InternalError(format!(
                            "failed to parse param placeholder '{}'",
                            param
                        ))
                    })
                })
            })
            .collect::<Result<Vec<_>, _>>()?;

        param_types.sort_by(|(a, _), (b, _)| a.cmp(b));
        Ok(param_types.into_iter().map(|(_, ty)| ty).collect())
    }

    /// Asks the [`TypeInferencer`] for a hashmap of literal types, validating that that are all `Scalar` types.
    fn literal_types(&self) -> Result<Vec<(EqlValue, &'ast ast::Expr)>, EqlMapperError> {
        let inferencer = self.inferencer.borrow();
        let literal_nodes = inferencer.literal_nodes();
        let literals: Vec<(EqlValue, &'ast ast::Expr)> = literal_nodes
            .iter()
            .map(|node_key| match inferencer.get_type_by_node_key(node_key) {
                Some(ty) => {
                    assert!(ty.is_fully_resolved(&self.registry.borrow()));

                    if let unifier::Type::Constructor(unifier::Constructor::Value(
                        eql_ty @ unifier::Value::Eql(_),
                    )) = &ty
                    {
                        match node_key.get_as::<ast::Expr>() {
                            Some(expr) => Ok(Some((EqlValue::try_from(eql_ty)?, expr))),
                            None => Err(EqlMapperError::InternalError(String::from(
                                "could not resolve literal node",
                            ))),
                        }
                    } else {
                        Ok(None)
                    }
                }
                None => Err(EqlMapperError::InternalError(String::from(
                    "failed to get type of literal node",
                ))),
            })
            .filter_map(Result::transpose)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(literals)
    }
}

impl<'ast> TypedStatement<'ast> {
    /// Some statements do not require transformation and this means the application can choose to skip the
    /// transformation step (which would be a no-op) and save come CPU cycles.
    ///
    /// Note: this check is conservative with respect to params. Some kinds of encrypted params will not require
    /// statement transformation but we do not currently track that information anywhere so instead we assume the all
    /// potentially require AST edits.
    pub fn requires_transform(&self) -> bool {
        // if there are any literals that require encryption, or any params that require encryption.
        !self.literals.is_empty()
            || self
                .params
                .iter()
                .any(|value_ty| matches!(value_ty, Value::Eql(_)))
    }

    /// Transforms the SQL statement by replacing all plaintext literals with EQL equivalents.
    pub fn transform(
        &self,
        encrypted_literals: HashMap<&'ast ast::Expr, ast::Expr>,
    ) -> Result<Statement, EqlMapperError> {
        for (_, target) in self.literals.iter() {
            if !encrypted_literals.contains_key(target) {
                return Err(EqlMapperError::Transform(String::from("encrypted literals refers to a literal node which is not present in the SQL statement")));
            }
        }

        self.statement
            .apply_transform(&mut EncryptedStatement::new(encrypted_literals))
    }
}

#[derive(Debug)]
struct EncryptedStatement<'ast> {
    encrypted_literals: HashMap<&'ast ast::Expr, ast::Expr>,
}

impl<'ast> EncryptedStatement<'ast> {
    fn new(encrypted_literals: HashMap<&'ast ast::Expr, ast::Expr>) -> Self {
        Self { encrypted_literals }
    }
}

/// Updates all [`Expr::Value`] nodes that:
///
/// 1. do not contain a [`Value::Placeholder`], and
/// 2. have been marked for replacement
impl<'ast> Transform<'ast> for EncryptedStatement<'ast> {
    type Error = EqlMapperError;

    fn transform<N: Visitable>(
        &mut self,
        original_node: &'ast N,
        mut new_node: N,
    ) -> Result<N, Self::Error> {
        if let Some(target_value) = new_node.downcast_mut::<ast::Expr>() {
            match original_node.downcast_ref::<ast::Expr>() {
                Some(original_value) => match original_value {
                    ast::Expr::Value(ast::Value::Placeholder(_)) => {
                        return Err(EqlMapperError::InternalError(
                            "attempt was made to update placeholder with literal".to_string(),
                        ));
                    }

                    _ => {
                        if let Some(replacement) = self.encrypted_literals.remove(original_value) {
                            *target_value = replacement;
                        }
                    }
                },
                None => {
                    return Err(EqlMapperError::Transform(String::from(
                        "Could not resolve literal node",
                    )));
                }
            }
        }

        Ok(new_node)
    }
}

/// [`Visitor`] implememtation that composes the [`Scope`] visitor, the [`Importer`] and the [`TypeInferencer`]
/// visitors.
impl<'ast> Visitor<'ast> for EqlMapper<'ast> {
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
    use std::collections::HashMap;

    use pretty_assertions::assert_eq;

    use crate::{
        schema, Dep, EqlValue, NativeValue, Projection, ProjectionColumn, TableColumn, Value,
    };

    use sqlparser::{
        ast::{self as ast, Statement},
        dialect::PostgreSqlDialect,
        parser::Parser,
    };

    use super::type_check;

    fn parse(statement: &'static str) -> Statement {
        Parser::parse_sql(&PostgreSqlDialect {}, statement).unwrap()[0].clone()
    }

    fn id(ident: &str) -> ast::Ident {
        ast::Ident::from(ident)
    }

    macro_rules! col {
        ((NATIVE)) => {
            ProjectionColumn {
                ty: Value::Native(NativeValue(None)),
                alias: None,
            }
        };

        ((NATIVE as $alias:ident)) => {
            ProjectionColumn {
                ty: Value::Native(NativeValue(None)),
                alias: Some(id(stringify!($alias))),
            }
        };

        ((NATIVE($table:ident . $column:ident))) => {
            ProjectionColumn {
                ty: Value::Native(NativeValue(Some(TableColumn {
                    table: id(stringify!($table)),
                    column: id(stringify!($column)),
                }))),
                alias: None,
            }
        };

        ((NATIVE($table:ident . $column:ident) as $alias:ident)) => {
            ProjectionColumn {
                ty: Value::Native(NativeValue(Some(TableColumn {
                    table: id(stringify!($table)),
                    column: id(stringify!($column)),
                }))),
                alias: Some(id(stringify!($alias))),
            }
        };

        ((EQL($table:ident . $column:ident))) => {
            ProjectionColumn {
                ty: Value::Eql(EqlValue::from((stringify!($table), stringify!($column)))),
                alias: None,
            }
        };

        ((EQL($table:ident . $column:ident) as $alias:ident)) => {
            ProjectionColumn {
                ty: Value::Eql(EqlValue(TableColumn {
                    table: id(stringify!($table)),
                    column: id(stringify!($column)),
                })),
                alias: Some(id(stringify!($alias))),
            }
        };
    }

    macro_rules! projection {
        [$($column:tt),*] => { Projection::new(vec![$(col!($column)),*]) };
    }

    #[test]
    fn basic() {
        let _ = tracing_subscriber::fmt::try_init();
        let schema = Dep::new(schema! {
            tables: {
                users: {
                    id (PK),
                    email,
                    first_name,
                }
            }
        });

        let statement = parse("select email from users");

        match type_check(&schema, &statement) {
            Ok(typed) => {
                assert_eq!(
                    typed.statement_type,
                    Some(projection![(NATIVE(users.email) as email)])
                )
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn select_columns_from_multiple_tables() {
        let _ = tracing_subscriber::fmt::try_init();
        let schema = Dep::new(schema! {
            tables: {
                users: {
                    id (PK),
                    email (EQL),
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

        match type_check(&schema, &statement) {
            Ok(typed) => {
                assert_eq!(
                    typed.statement_type,
                    Some(projection![(EQL(users.email) as email)])
                )
            }
            Err(err) => panic!("type check failed: {err}"),
        }
    }

    #[test]
    fn select_columns_from_subquery() {
        let _ = tracing_subscriber::fmt::try_init();
        let schema = Dep::new(schema! {
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
                    description (EQL),
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

        let Ok(typed) = type_check(&schema, &statement) else {
            panic!("type check failed")
        };

        assert_eq!(
            typed.statement_type,
            Some(projection![
                (NATIVE as user_id),
                (NATIVE as todo_list_item_id),
                (EQL(todo_list_items.description) as todo_list_item_description)
            ])
        );
    }

    #[test]
    fn wildcard_expansion() {
        let _ = tracing_subscriber::fmt::try_init();
        let schema = Dep::new(schema! {
            tables: {
                users: {
                    id,
                    email (EQL),
                }
                todo_lists: {
                    id,
                    owner_id,
                    secret (EQL),
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

        let typed = match type_check(&schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.statement_type,
            Some(projection![
                (NATIVE(users.id) as id),
                (EQL(users.email) as email),
                (NATIVE(todo_lists.id) as id),
                (NATIVE(todo_lists.owner_id) as owner_id),
                (EQL(todo_lists.secret) as secret)
            ])
        );
    }

    #[test]
    fn correlated_subquery() {
        let schema = Dep::new(schema! {
            tables: {
                employees: {
                    id,
                    first_name,
                    last_name,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                select
                    first_name,
                    last_name,
                    salary
                from
                    employees
                where
                    salary > (select salary from employees where first_name = 'Alice')
            "#,
        );

        let typed = match type_check(&schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.statement_type,
            Some(projection![
                (NATIVE(employees.first_name) as first_name),
                (NATIVE(employees.last_name) as last_name),
                (EQL(employees.salary) as salary)
            ])
        );
    }

    #[test]
    fn window_function() {
        let _ = tracing_subscriber::fmt::try_init();

        let schema = Dep::new(schema! {
            tables: {
                employees: {
                    id,
                    first_name,
                    last_name,
                    department_name,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                select
                    first_name,
                    last_name,
                    department_name,
                    salary,
                    rank() over (partition by department_name order by salary desc)
                from
                   employees
            "#,
        );

        let typed = match type_check(&schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.statement_type,
            Some(projection![
                (NATIVE(employees.first_name) as first_name),
                (NATIVE(employees.last_name) as last_name),
                (NATIVE(employees.department_name) as department_name),
                (EQL(employees.salary) as salary),
                (NATIVE as rank)
            ])
        );
    }

    #[test]
    fn window_function_with_forward_reference() {
        let _ = tracing_subscriber::fmt::try_init();

        let schema = Dep::new(schema! {
            tables: {
                employees: {
                    id,
                    first_name,
                    last_name,
                    department_name,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                select
                    first_name,
                    last_name,
                    department_name,
                    salary,
                    rank() over w
                from
                   employees
                window w AS (partition BY department_name order by salary desc);
            "#,
        );

        let typed = match type_check(&schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.statement_type,
            Some(projection![
                (NATIVE(employees.first_name) as first_name),
                (NATIVE(employees.last_name) as last_name),
                (NATIVE(employees.department_name) as department_name),
                (EQL(employees.salary) as salary),
                (NATIVE as rank)
            ])
        );
    }

    #[test]
    fn common_table_expressions() {
        let _ = tracing_subscriber::fmt::try_init();

        let schema = Dep::new(schema! {
            tables: {
                employees: {
                    id,
                    first_name,
                    last_name,
                    department_name,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                with salaries_by_department as (
                    select
                        first_name,
                        last_name,
                        department_name,
                        salary,
                        rank() over w
                    from
                    employees
                    window w AS (partition BY department_name order by salary desc)
                )
                select * from salaries_by_department
            "#,
        );

        let typed = match type_check(&schema, &statement) {
            Ok(typed) => typed,
            Err(err) => {
                // eprintln!("Error: {}", err, err.source());
                panic!("type check failed: {:#?}", err)
            }
        };

        assert_eq!(
            typed.statement_type,
            Some(projection![
                (NATIVE(employees.first_name) as first_name),
                (NATIVE(employees.last_name) as last_name),
                (NATIVE(employees.department_name) as department_name),
                (EQL(employees.salary) as salary),
                (NATIVE as rank)
            ])
        );
    }

    #[test]
    fn aggregates() {
        let _ = tracing_subscriber::fmt::try_init();

        let schema = Dep::new(schema! {
            tables: {
                employees: {
                    id,
                    department,
                    age,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                select
                    max(age),
                    min(salary)
                from employees
                group by department
            "#,
        );

        let typed = match type_check(&schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.statement_type,
            Some(projection![
                (NATIVE(employees.age) as max),
                (EQL(employees.salary) as min)
            ])
        );
    }

    #[test]
    fn insert() {
        let _ = tracing_subscriber::fmt::try_init();

        let schema = Dep::new(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    department,
                    age,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                insert into employees (name, department, age, salary)
                    values ('Alice', 'Engineering', 28, 180000)
            "#,
        );

        let typed = match type_check(&schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(typed.statement_type, Some(Projection::Empty));
    }

    #[test]
    fn insert_with_returning_clause() {
        let _ = tracing_subscriber::fmt::try_init();

        let schema = Dep::new(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    department,
                    age,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                insert into employees (name, department, age, salary)
                    values ('Alice', 'Engineering', 28, 180000)
                    returning *
            "#,
        );

        let typed = match type_check(&schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.statement_type,
            Some(projection![
                (NATIVE(employees.id) as id),
                (NATIVE(employees.name) as name),
                (NATIVE(employees.department) as department),
                (NATIVE(employees.age) as age),
                (EQL(employees.salary) as salary)
            ])
        );
    }

    #[test]
    fn update() {
        let _ = tracing_subscriber::fmt::try_init();

        let schema = Dep::new(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    department,
                    age,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                update employees set name = 'Alice', salary = 18000 where id = 123
            "#,
        );

        let typed = match type_check(&schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(typed.statement_type, Some(Projection::Empty));
    }

    #[test]
    fn update_with_returning_clause() {
        let _ = tracing_subscriber::fmt::try_init();

        let schema = Dep::new(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    department,
                    age,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                update employees set name = 'Alice', salary = 18000 where id = 123 returning *
            "#,
        );

        let typed = match type_check(&schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.statement_type,
            Some(projection![
                (NATIVE(employees.id) as id),
                (NATIVE(employees.name) as name),
                (NATIVE(employees.department) as department),
                (NATIVE(employees.age) as age),
                (EQL(employees.salary) as salary)
            ])
        );
    }

    #[test]
    fn delete() {
        let _ = tracing_subscriber::fmt::try_init();

        let schema = Dep::new(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    department,
                    age,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                delete from employees where salary > 200000
            "#,
        );

        let typed = match type_check(&schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(typed.statement_type, Some(Projection::Empty));
    }

    #[test]
    fn delete_with_returning_clause() {
        let _ = tracing_subscriber::fmt::try_init();

        let schema = Dep::new(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    department,
                    age,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                delete from employees where salary > 200000 returning *
            "#,
        );

        let typed = match type_check(&schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.statement_type,
            Some(projection![
                (NATIVE(employees.id) as id),
                (NATIVE(employees.name) as name),
                (NATIVE(employees.department) as department),
                (NATIVE(employees.age) as age),
                (EQL(employees.salary) as salary)
            ])
        );
    }

    #[test]
    fn select_with_literal_subsitution() {
        let _ = tracing_subscriber::fmt::try_init();

        let schema = Dep::new(schema! {
            tables: {
                employees: {
                    id,
                    name,
                    department,
                    age,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                select * from employees where salary > 200000
            "#,
        );

        let typed = match type_check(&schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert!(typed.literals.contains(&(
            EqlValue(TableColumn {
                table: id("employees"),
                column: id("salary")
            }),
            &ast::Expr::Value(ast::Value::Number(200000.into(), false))
        )));

        let transformed_statement = match typed.transform(HashMap::from_iter([(
            &ast::Expr::Value(ast::Value::Number(200000.into(), false)),
            ast::Expr::Value(ast::Value::SingleQuotedString("ENCRYPTED".into())),
        )])) {
            Ok(transformed_statement) => transformed_statement,
            Err(err) => panic!("statement transformation failed: {}", err),
        };

        // This type checks the transformed statement so we can get hold of the encrypted literal.
        let typed = match type_check(&schema, &transformed_statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert!(typed.literals.contains(&(
            EqlValue(TableColumn {
                table: id("employees"),
                column: id("salary")
            }),
            &ast::Expr::Value(ast::Value::SingleQuotedString("ENCRYPTED".into())),
        )));
    }

    #[test]
    fn pathologically_complex_sql_statement() {
        let _ = tracing_subscriber::fmt::try_init();

        let schema = Dep::new(schema! {
            tables: {
                employees: {
                    id,
                    department_id,
                    name,
                    salary (EQL),
                }
            }
        });

        let statement = parse(
            r#"
                select * from
                (select min(salary) as min_salary from employees) as x
                inner join (
                    (
                        select salary as y from employees
                            where salary < (select min(foo) from (
                                select salary as foo from employees
                            )
                        )
                    )
                    union
                    (
                        select salary as y from employees
                            where salary >= (select min(max(foo)) from (
                                select salary as foo from employees
                            )
                        )
                    )
                ) as holy_joins_batman on x.min_salary = holy_joins_batman.y
                inner join employees as e on (e.salary = holy_joins_batman.y)
            "#,
        );

        let typed = match type_check(&schema, &statement) {
            Ok(typed) => typed,
            Err(err) => panic!("type check failed: {:#?}", err),
        };

        assert_eq!(
            typed.statement_type,
            Some(projection![
                (EQL(employees.salary) as min_salary),
                (EQL(employees.salary) as y),
                (NATIVE(employees.id) as id),
                (NATIVE(employees.department_id) as department_id),
                (NATIVE(employees.name) as name),
                (EQL(employees.salary) as salary)
            ])
        );
    }
}
