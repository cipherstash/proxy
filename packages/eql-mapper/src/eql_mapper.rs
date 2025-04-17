use super::importer::{ImportError, Importer};
use crate::{
    inference::{TypeError, TypeInferencer},
    unifier::{EqlValue, Unifier},
    DepMut, Param, ParamError, ScopeError, ScopeTracker, TableResolver, Type, TypeRegistry,
    TypedStatement, Value,
};
use sqlparser::ast::{self as ast, Statement};
use sqltk::{Break, NodeKey, Visitable, Visitor};
use std::{
    cell::RefCell, collections::HashMap, marker::PhantomData, ops::ControlFlow, rc::Rc, sync::Arc,
};
use tracing::{span, Level};

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
    resolver: Arc<TableResolver>,
    statement: &'ast Statement,
) -> Result<TypedStatement<'ast>, EqlMapperError> {
    let mut mapper = EqlMapper::<'ast>::new_with_resolver(resolver);
    match statement.accept(&mut mapper) {
        ControlFlow::Continue(()) => {
            let build = || -> Result<TypedStatement, EqlMapperError> {
                let projection = mapper.projection_type(statement);
                let params = mapper.param_types();
                let literals = mapper.literal_types();
                let node_types = mapper.node_types();

                let span = span!(
                    Level::DEBUG,
                    "type_check",
                    projection = ?projection,
                    params = ?params,
                    literals = ?literals,
                    node_types = ?node_types
                );

                let _guard = span.enter();

                Ok(TypedStatement {
                    statement,
                    projection: projection?,
                    params: params?,
                    literals: literals?,
                    node_types: Arc::new(node_types?),
                })
            };

            build().map_err(|err| {
                let span = span!(
                    Level::ERROR,
                    "type_check",
                    err = ?err
                );

                let _guard = span.enter();

                err
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

/// The error type returned by various functions in the `eql_mapper` crate.
#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum EqlMapperError {
    #[error("Error during SQL transformation: {}", _0)]
    Transform(String),

    #[error("Internal error: {}", _0)]
    InternalError(String),

    /// A lexical scope error
    #[error(transparent)]
    Scope(#[from] ScopeError),

    /// An error when attempting to import a table or table-column from the database schema
    #[error(transparent)]
    Import(#[from] ImportError),

    /// A type error encountered during type checking
    #[error(transparent)]
    Type(#[from] TypeError),

    /// A [`Param`] could not be parsed
    #[error(transparent)]
    Param(#[from] ParamError),
}

/// `EqlMapper` can safely convert a SQL statement into an equivalent statement where all of the plaintext literals have
/// been converted to EQL payloads containing the encrypted literal and/or encrypted representations of those literals.
struct EqlMapper<'ast> {
    scope_tracker: Rc<RefCell<ScopeTracker<'ast>>>,
    importer: Rc<RefCell<Importer<'ast>>>,
    inferencer: Rc<RefCell<TypeInferencer<'ast>>>,
    registry: Rc<RefCell<TypeRegistry<'ast>>>,
    unifier: Rc<RefCell<Unifier<'ast>>>,
    _ast: PhantomData<&'ast ()>,
}

impl<'ast> EqlMapper<'ast> {
    /// Build an `EqlMapper`, initialising all the other visitor implementations that it depends on.
    fn new_with_resolver(table_resolver: Arc<TableResolver>) -> Self {
        let registry = DepMut::new(TypeRegistry::new());
        let scope_tracker = DepMut::new(ScopeTracker::new(&registry));
        let importer = DepMut::new(Importer::new(
            table_resolver.clone(),
            &registry,
            &scope_tracker,
        ));
        let unifier = DepMut::new(Unifier::new(&registry));

        let inferencer = DepMut::new(TypeInferencer::new(
            table_resolver.clone(),
            &scope_tracker,
            &registry,
            &unifier,
        ));

        Self {
            scope_tracker: scope_tracker.into(),
            importer: importer.into(),
            inferencer: inferencer.into(),
            registry: registry.into(),
            unifier: unifier.into(),
            _ast: PhantomData,
        }
    }

    fn projection_type(
        &self,
        statement: &'ast Statement,
    ) -> Result<crate::Projection, EqlMapperError> {
        let mut unifier = self.unifier.borrow_mut();

        let ty = unifier.lookup_type_by_node(statement);
        Ok(ty.resolved_as::<crate::Projection>(&mut unifier)?)
    }

    fn param_types(&self) -> Result<Vec<(Param, Value)>, EqlMapperError> {
        let params = self.inferencer.borrow().param_types()?;

        let params = params
            .into_iter()
            .map(|(p, ty)| -> Result<(Param, Value), EqlMapperError> {
                match ty.resolved(&mut self.unifier.borrow_mut())? {
                    Type::Value(value) => Ok((p, value)),
                    other => Err(TypeError::Expected(format!(
                        "expected param '{}' to resolve to a scalar type but got '{}'",
                        p, other
                    )))?,
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(params)
    }

    /// Asks the [`TypeInferencer`] for a hashmap of literal types, validating that they are all `Scalar` types.
    fn literal_types(&self) -> Result<Vec<(EqlValue, &'ast ast::Value)>, EqlMapperError> {
        let iter = {
            let reg = self.registry.borrow();
            reg.get_nodes_and_types::<ast::Value>().into_iter()
        };
        let literal_nodes: Vec<(EqlValue, &'ast ast::Value)> = iter
            .map(
                |(node, ty)| -> Result<Option<(EqlValue, &'ast ast::Value)>, TypeError> {
                    if let Some(ty) = ty {
                        if let crate::Type::Value(crate::Value::Eql(eql_value)) =
                            &ty.resolved(&mut self.unifier.borrow_mut())?
                        {
                            return Ok(Some((eql_value.clone(), node)));
                        }
                        Ok(None)
                    } else {
                        Err(TypeError::InternalError(format!(
                            "could not resolve type for literal node: {}",
                            node.to_string()
                        )))
                    }
                },
            )
            .filter_map(Result::transpose)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(literal_nodes)
    }

    fn node_types(&self) -> Result<HashMap<NodeKey<'ast>, Type>, EqlMapperError> {
        let inferencer = self.inferencer.borrow();
        let node_types = inferencer.node_types();

        let mut resolved_node_types: HashMap<NodeKey<'ast>, Type> = HashMap::new();
        for (key, ty) in node_types {
            match ty {
                Some(ty) => resolved_node_types.insert(key, ty.resolved(&mut self.unifier.borrow_mut())?),
                None => return Err(EqlMapperError::InternalError(format!("unresolved type for node")))
            };
        }

        Ok(resolved_node_types)
    }
}

/// [`Visitor`] implementation that composes the [`ScopeTracker`] visitor, the [`Importer`] and the [`TypeInferencer`]
/// visitors.
impl<'ast> Visitor<'ast> for EqlMapper<'ast> {
    type Error = EqlMapperError;

    fn enter<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        self.scope_tracker
            .borrow_mut()
            .enter(node)
            .map_break(Break::convert)?;

        self.importer
            .borrow_mut()
            .enter(node)
            .map_break(Break::convert)?;

        self.inferencer
            .borrow_mut()
            .enter(node)
            .map_break(Break::convert)?;

        ControlFlow::Continue(())
    }

    fn exit<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        self.inferencer
            .borrow_mut()
            .exit(node)
            .map_break(Break::convert)?;

        self.importer
            .borrow_mut()
            .exit(node)
            .map_break(Break::convert)?;

        self.scope_tracker
            .borrow_mut()
            .exit(node)
            .map_break(Break::convert)?;

        ControlFlow::Continue(())
    }
}
