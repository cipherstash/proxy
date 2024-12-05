use std::{
    cell::RefCell, collections::HashMap, marker::PhantomData, ops::ControlFlow, rc::Rc, sync::Arc,
};

use sqlparser::ast::{Expr, Statement, Value};
use sqltk::{convert_control_flow, Break, Semantic, Transform, Transformable, Visitable, Visitor};

use crate::{
    inference::{unifier, TypeError, TypeInferencer},
    Dep, DepMut, EqlColumn, NodeKey, Projection, ProjectionColumn, Scalar, Schema, Scope,
    ScopeError, Type, TypeRegistry,
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
            mapper.inferencer.borrow().try_resolve_all_types()?;

            Ok(TypedStatement {
                statement,
                params: mapper.param_types()?,
                literals: mapper.literal_types()?,
                node_types: mapper.node_types()?,
            })
        }
        ControlFlow::Break(Break::Err(err)) => Err(err),
        ControlFlow::Break(_) => Err(EqlMapperError::InternalError(String::from(
            "unexpected Break value in type_check",
        ))),
    }
}

/// The result returned from a successful call to [`type_check`].
#[derive(Debug)]
pub struct TypedStatement<'ast> {
    /// The SQL statement which was type-checked against the schema.
    pub statement: &'ast Statement,

    /// The types of all params discovered from [`Value::Placeholder`] nodes in the SQL statement.
    pub params: Vec<Scalar>,

    /// The types and values of all literals from the SQL statement.
    pub literals: Vec<(EqlColumn, &'ast Value)>,

    /// The types of all semantically interesting nodes with the AST.
    node_types: HashMap<NodeKey<'ast>, Type>,
}

/// The error type returned by various functions in the `eql_mapper` crate.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum EqlMapperError {
    /// A lexical scope error
    #[error(transparent)]
    Scope(#[from] ScopeError),

    /// An error when attempting to import a table or table-column from the database schema
    #[error(transparent)]
    Import(#[from] ImportError),

    /// A type error encountered during type checking
    #[error(transparent)]
    Type(#[from] TypeError),

    #[error("Error during SQL transformation: {}", _0)]
    Transform(String),

    #[error("Internal error: {}", _0)]
    InternalError(String),

    #[error("Unsupported value variant: {}", _0)]
    UnsupportedValueVariant(String),
}

/// `EqlMapper` can safely convert a SQL statement into an equivalent statement where all of the plaintext literals have
/// been converted to EQL payloads containing the encrypted literal and/or encrypted representations of those literals.
#[derive(Debug)]
struct EqlMapper<'ast> {
    scope: Rc<RefCell<Scope>>,
    importer: Rc<RefCell<Importer<'ast>>>,
    inferencer: Rc<RefCell<TypeInferencer<'ast>>>,
    _ast: PhantomData<&'ast ()>,
}

impl<'ast> EqlMapper<'ast> {
    /// Build an `EqlMapper`, initialising all the other visitor implementations that it depends on.
    fn new_from_schema(schema: impl Into<Arc<Schema>>) -> Self {
        let schema = Dep::from(schema.into());
        let scope = DepMut::new(Scope::new());
        let registry = DepMut::new(TypeRegistry::new());
        let importer = DepMut::new(Importer::new(&schema, &registry, &scope));
        let inferencer = DepMut::new(TypeInferencer::new(&schema, &scope, &registry));

        Self {
            scope: scope.into(),
            importer: importer.into(),
            inferencer: inferencer.into(),
            _ast: PhantomData,
        }
    }

    /// Asks the [`TypeInferencer`] for a hashmap of node types.
    fn node_types(&self) -> Result<HashMap<NodeKey<'ast>, Type>, EqlMapperError> {
        let node_types = self.inferencer.borrow().node_types()?;

        node_types
            .iter()
            .map(|(key, ty)| Type::try_from(ty).map(|ty| (key.clone(), ty)))
            .collect::<Result<HashMap<_, _>, _>>()
    }

    /// Asks the [`TypeInferencer`] for a hashmap of parameter types.
    fn param_types(&self) -> Result<Vec<Scalar>, EqlMapperError> {
        let param_types = self.inferencer.borrow().param_types()?;

        let mut param_types: Vec<(i32, Scalar)> = param_types
            .iter()
            .map(|(param, ty)| {
                Scalar::try_from(ty).and_then(|ty| {
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
    fn literal_types(&self) -> Result<Vec<(EqlColumn, &'ast Value)>, EqlMapperError> {
        let inferencer = self.inferencer.borrow();
        let literal_nodes = inferencer.literal_nodes();
        let literals: Vec<(EqlColumn, &'ast Value)> = literal_nodes
            .iter()
            .map(|node_key| match inferencer.get_type_by_node_key(node_key) {
                Some(ty) => {
                    if let unifier::Type(
                        unifier::Def::Constructor(unifier::Constructor::Scalar(scalar_ty)),
                        unifier::Status::Resolved,
                    ) = &*ty.borrow()
                    {
                        match node_key.get_as::<Value>() {
                            Some(value) => Ok((EqlColumn::try_from(&**scalar_ty)?, value)),
                            None => Err(EqlMapperError::InternalError(String::from(
                                "could not resolve literal node",
                            ))),
                        }
                    } else {
                        Err(EqlMapperError::InternalError(
                            "literal is not a scalar type".to_string(),
                        ))
                    }
                }
                None => Err(EqlMapperError::InternalError(String::from(
                    "failed to get type of literal node",
                ))),
            })
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
                .any(|scalar_ty| matches!(scalar_ty, Scalar::EqlColumn(_)))
    }

    /// Tries to get a [`Value`] (a literal) from `self`.
    ///
    /// This method can fail because it cannot be proven at the type-level that [`NodeKey`] refers to a `Value`.
    pub fn try_get_literal(&self, node_key: &NodeKey<'ast>) -> Result<&'ast Value, EqlMapperError> {
        match node_key.get_as::<Expr>() {
            Some(Expr::Value(value)) => Ok(value),
            Some(_) => Err(EqlMapperError::InternalError(
                "try_get_literal: wrong expression type".to_string(),
            )),
            None => Err(EqlMapperError::InternalError(
                "try_get_literal: failed to get literal".to_string(),
            )),
        }
    }

    /// Gets the [`Type`] associated with a semantically-interesting AST node.
    pub fn get_type<N: Semantic>(&self, node: &'ast N) -> Option<&Type> {
        self.node_types.get(&NodeKey::new(node))
    }

    /// Gets the projection associated with a SQL statement.
    ///
    /// Not all statments have a projection, so the result is wrapped in an [`Option`].
    pub fn get_projection_columns(&self) -> Option<&[ProjectionColumn]> {
        match self.node_types.get(&NodeKey::new(self.statement)) {
            Some(ty) => match ty {
                Type::Projection(Projection(columns)) => Some(columns.as_slice()),
                _ => None,
            },
            None => None,
        }
    }

    /// Transforms the SQL statement by replacing all plaintext literals with EQL equivalents.
    pub fn transform(
        &self,
        encrypted_literals: HashMap<&'ast Value, Value>,
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
    encrypted_literals: HashMap<&'ast Value, Value>,
}

impl<'ast> EncryptedStatement<'ast> {
    fn new(encrypted_literals: HashMap<&'ast Value, Value>) -> Self {
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
        if let Some(target_value) = new_node.downcast_mut::<Value>() {
            match original_node.downcast_ref::<Value>() {
                Some(original_value) => match original_value {
                    Value::Placeholder(_) => {
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
        convert_control_flow(self.scope.borrow_mut().enter(node))?;
        convert_control_flow(self.importer.borrow_mut().enter(node))?;
        convert_control_flow(self.inferencer.borrow_mut().enter(node))?;

        ControlFlow::Continue(())
    }

    fn exit<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        convert_control_flow(self.inferencer.borrow_mut().exit(node))?;
        convert_control_flow(self.importer.borrow_mut().exit(node))?;
        convert_control_flow(self.scope.borrow_mut().exit(node))?;

        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod test {
    use pretty_assertions::assert_eq;

    use sqlparser::{
        ast::{Ident, Statement},
        dialect::PostgreSqlDialect,
        parser::Parser,
    };

    // use crate::{TableColumn, Dep, make_schema, type_check};
    use crate::*;

    fn parse(statement: &'static str) -> Statement {
        Parser::parse_sql(&PostgreSqlDialect {}, statement).unwrap()[0].clone()
    }

    fn id(ident: &str) -> Ident {
        Ident::from(ident)
    }

    #[test]
    fn basic() {
        let schema = Dep::new(make_schema! {
            tables: {
                users: {
                    id (PK),
                    email,
                    first_name,
                }
            }
        });

        let statement = parse("select email from users");

        assert!(type_check(&schema, &statement).is_ok());
    }

    #[test]
    fn select_columns_from_multiple_tables() {
        let schema = Dep::new(make_schema! {
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

        let Ok(typed) = type_check(&schema, &statement) else {
            panic!("type check failed")
        };

        assert_eq!(
            typed.get_type(&statement),
            Some(&Type::Projection(Projection(vec![ProjectionColumn {
                ty: ProjectionColumnType::Scalar(Scalar::EqlColumn(EqlColumn(TableColumn {
                    table: id("users"),
                    column: id("email")
                }))),
                alias: Some(id("email"))
            }])))
        )
    }

    #[test]
    fn select_columns_from_subquery() {
        let schema = Dep::new(make_schema! {
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

        let Ok(typed) = type_check(&schema, &statement) else {
            panic!("type check failed")
        };

        assert_eq!(
            typed.get_type(&statement),
            Some(&Type::Projection(Projection(vec![
                ProjectionColumn {
                    ty: ProjectionColumnType::Scalar(Scalar::Native(None)),
                    alias: Some(id("user_id"))
                },
                ProjectionColumn {
                    ty: ProjectionColumnType::Scalar(Scalar::Native(None)),
                    alias: Some(id("todo_list_item_id"))
                },
                ProjectionColumn {
                    ty: ProjectionColumnType::Scalar(Scalar::EqlColumn(EqlColumn(TableColumn {
                        table: id("todo_list_items"),
                        column: id("description")
                    }))),
                    alias: Some(id("todo_list_item_description"))
                }
            ])))
        );
    }

    #[test]
    #[ignore]
    fn wildcard_expansion() {
        let schema = Dep::new(make_schema! {
            tables: {
                users: {
                    id,
                    email (ENCRYPTED),
                }
                todo_lists: {
                    id,
                    secret (ENCRYPTED),
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

        let Ok(typed) = type_check(&schema, &statement) else {
            panic!("type check failed")
        };

        assert_eq!(
            typed.get_type(&statement),
            Some(&Type::Projection(Projection(vec![
                ProjectionColumn {
                    ty: ProjectionColumnType::Scalar(Scalar::Native(Some(TableColumn {
                        table: id("users"),
                        column: id("id")
                    }))),
                    alias: None
                },
                ProjectionColumn {
                    ty: ProjectionColumnType::Scalar(Scalar::EqlColumn(EqlColumn(TableColumn {
                        table: id("users"),
                        column: id("email")
                    }))),
                    alias: None
                },
                ProjectionColumn {
                    ty: ProjectionColumnType::Scalar(Scalar::Native(Some(TableColumn {
                        table: id("todo_lists"),
                        column: id("id")
                    }))),
                    alias: None
                },
                ProjectionColumn {
                    ty: ProjectionColumnType::Scalar(Scalar::EqlColumn(EqlColumn(TableColumn {
                        table: id("todo_lists"),
                        column: id("secret")
                    }))),
                    alias: None
                },
            ])))
        );
    }
}
