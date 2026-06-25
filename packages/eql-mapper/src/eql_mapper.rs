use super::importer::{ImportError, Importer};
use crate::{
    inference::{TypeError, TypeInferencer},
    unifier::{EqlTerm, Projection, Type, Unifier, Value},
    DepMut, EmptyIndexResolver, IndexResolver, Param, ParamError, ScopeError, ScopeTracker,
    TableResolver, TypeCheckedStatement, TypeRegistry,
};
use sqltk::parser::ast::{self as ast, Statement};
use sqltk::{Break, NodeKey, Visitable, Visitor};
use std::{
    cell::RefCell, collections::HashMap, marker::PhantomData, ops::ControlFlow, rc::Rc, sync::Arc,
};
use tracing::{event, span, Level};

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
/// A successful type check will return a [`TypeCheckedStatement`] which can be interrogated to discover the required params
/// and their types, the types and plaintext values of all literals, and an optional projection type (the optionality
/// depending on the specific statement).
///
/// Invoking [`TypeCheckedStatement::transform`] will return an updated [`Statement`] where all plaintext literals have been
/// replaced with their EQL (encrypted) equivalent and specific SQL operators and functions will have been rewritten to
/// invoke the EQL equivalents.
///
/// An [`EqlMapperError`] is returned if type checking fails.
pub fn type_check<'ast>(
    resolver: Arc<TableResolver>,
    statement: &'ast Statement,
) -> Result<TypeCheckedStatement<'ast>, EqlMapperError> {
    type_check_with_indexes(resolver, statement, Arc::new(EmptyIndexResolver))
}

/// Like [`type_check`] but additionally takes an [`IndexResolver`] that exposes
/// the concrete encrypted-index types of `(table, column)` pairs to the SQL
/// *transformation* stage.
///
/// Type inference and unification are identical to [`type_check`]: the
/// [`IndexResolver`] is a side-channel consulted only by transformation rules
/// (via [`TypeCheckedStatement::transform`]) to choose index-specific target
/// functions. Passing [`EmptyIndexResolver`] is exactly equivalent to calling
/// [`type_check`].
pub fn type_check_with_indexes<'ast>(
    resolver: Arc<TableResolver>,
    statement: &'ast Statement,
    index_resolver: Arc<dyn IndexResolver>,
) -> Result<TypeCheckedStatement<'ast>, EqlMapperError> {
    let mut mapper = EqlMapper::<'ast>::new_with_resolver(resolver);
    match statement.accept(&mut mapper) {
        ControlFlow::Continue(()) => mapper.resolve(statement, index_resolver),
        ControlFlow::Break(Break::Err(err)) => Err(err),
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
    matches!(
        statement,
        Statement::Query(_)
            | Statement::Insert(_)
            | Statement::Update { .. }
            | Statement::Delete(_)
            | Statement::Merge { .. }
            | Statement::Prepare { .. }
            | Statement::Explain { .. }
    )
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
        let scope_tracker = DepMut::new(ScopeTracker::new());
        let importer = DepMut::new(Importer::new(
            table_resolver.clone(),
            &registry,
            &scope_tracker,
        ));
        let unifier = DepMut::new(Unifier::new(&registry));

        let inferencer = DepMut::new(TypeInferencer::new(
            table_resolver.clone(),
            &scope_tracker,
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

    pub fn resolve(
        self,
        statement: &'ast Statement,
        index_resolver: Arc<dyn IndexResolver>,
    ) -> Result<TypeCheckedStatement<'ast>, EqlMapperError> {
        let span_begin = span!(
            target: "eqlmapper::spans",
            Level::TRACE,
            "resolve",
            statement = %statement
        );

        let _guard = span_begin.enter();

        let _ = self
            .unifier
            .borrow_mut()
            .resolve_unresolved_associated_types();

        let _ = self.unifier.borrow_mut().resolve_unresolved_value_nodes();

        let projection = self.projection_type(statement);
        let params = self.param_types(&self.unifier.borrow());
        let literals = self.literal_types();
        let node_types = self.node_types();

        let combine_results =
            || -> Result<_, EqlMapperError> { Ok((projection?, params?, literals?, node_types?)) };

        match combine_results() {
            Ok((projection, params, literals, node_types)) => {
                // event!(
                //     target: "eql-mapper::EVENT_RESOLVE_OK",
                //     parent: &span_begin,
                //     Level::TRACE,
                //     projection = %&projection,
                //     params = %Fmt(&params),
                //     literals = %Fmt(&literals),
                //     node_types = %Fmt(&node_types)
                // );

                Ok(TypeCheckedStatement::new(
                    statement,
                    projection,
                    params,
                    literals,
                    Arc::new(node_types),
                    index_resolver,
                ))
            }
            Err(err) => {
                {
                    let unifier = &*self.unifier.borrow();
                    unifier.dump_all_nodes(statement);
                    unifier.dump_substitutions();
                }

                let projection = self.projection_type(statement);
                let params = self.param_types(&self.unifier.borrow());
                let literals = self.literal_types();
                let node_types = self.node_types();

                event!(
                    target: "eql-mapper::EVENT_RESOLVE_ERR",
                    parent: &span_begin,
                    Level::TRACE,
                    err = ?err,
                    projection = ?projection,
                    params = ?params,
                    literals = ?literals,
                    node_types = ?node_types
                );

                Err(err)
            }
        }
    }

    fn projection_type(&self, statement: &'ast Statement) -> Result<Projection, EqlMapperError> {
        let unifier = self.unifier.borrow();

        let ty = unifier.get_node_type(statement);
        let ty = ty.follow_tvars(&unifier);
        let projection = ty.resolved_as::<Projection>(&unifier)?;
        Ok(projection.flatten(&unifier)?)
    }

    fn param_types(&self, unifier: &Unifier<'ast>) -> Result<Vec<(Param, Value)>, EqlMapperError> {
        let params = self.registry.borrow().resolved_param_types(unifier)?;
        let (ste_vec_params, _) = self.ste_vec_term_rhs_keys(unifier);

        let params = params
            .into_iter()
            .map(|(p, ty)| -> Result<(Param, Value), EqlMapperError> {
                let ty = ty.follow_tvars(unifier);
                match &*ty {
                    Type::Value(value) => {
                        let value = reclassify_as_ste_vec_term_if(
                            value.clone(),
                            ste_vec_params.contains(&p),
                        );
                        Ok((p, value))
                    }
                    other => Err(TypeError::Expected(format!(
                        "expected param '{p}' to resolve to a scalar type but got '{other}'"
                    )))?,
                }
            })
            .collect::<Result<Vec<_>, _>>()?;

        Ok(params)
    }

    /// Collects the right-hand-side operand of every jsonb STE-vec *term*
    /// comparison — both *ordering* (`<`, `<=`, `>`, `>=`) and *equality*
    /// (`=`, `<>`) — whose left-hand side is a jsonb STE-vec element accessor
    /// (`->` / `->>` / `jsonb_path_query_first`).
    ///
    /// The returned sets identify the RHS values (params by [`Param`], literals
    /// by [`NodeKey`]) that must be encrypted as a STE-vec query term
    /// ([`EqlTerm::SteVecTerm`]) rather than a full/partial root payload.
    /// Ordering binds the term to `eql_v2.ore_cllw(...)` (`oc`); equality binds
    /// it to `eql_v2.eq_term(...)` (the XOR-aware `hm`/`oc` term the column's
    /// leaf carries). Both require the same `SteVecTerm` reclassification — the
    /// proxy's encrypt path emits whichever term the column's leaf carries.
    fn ste_vec_term_rhs_keys(
        &self,
        unifier: &Unifier<'ast>,
    ) -> (
        std::collections::HashSet<Param>,
        std::collections::HashSet<NodeKey<'ast>>,
    ) {
        use crate::ste_vec_ordering::{
            is_equality_operator, is_ordering_operator, is_ste_vec_accessor,
        };
        use sqltk::parser::ast::Expr;

        let mut params = std::collections::HashSet::new();
        let mut literals = std::collections::HashSet::new();

        let registry = self.registry.borrow();

        // `is_eql` mirrors the `RewriteJsonbSteVecOrdering` /
        // `RewriteJsonbSteVecEquality` rules' `is_eql_typed` check on both
        // operands, so the reclassification (which changes how the value is
        // encrypted) marks exactly the comparisons the SQL rewrites will
        // rewrite to `eql_v2.ore_cllw(...)` / `eql_v2.eq_term(...)`.
        let is_eql = |expr: &Expr| {
            registry
                .peek_node_type(expr)
                .map(|ty| matches!(&*ty.follow_tvars(unifier), Type::Value(Value::Eql(_))))
                .unwrap_or(false)
        };

        for (expr, _) in registry.get_nodes_and_types::<Expr>() {
            let Expr::BinaryOp { left, op, right } = expr else {
                continue;
            };

            if !(is_ordering_operator(op) || is_equality_operator(op))
                || !is_ste_vec_accessor(left)
                || !is_eql(left)
                || !is_eql(right)
            {
                continue;
            }

            if let Expr::Value(value_with_span) = &**right {
                match &value_with_span.value {
                    ast::Value::Placeholder(p) => {
                        if let Ok(param) = Param::try_from(p) {
                            params.insert(param);
                        }
                    }
                    other => {
                        literals.insert(NodeKey::new(other));
                    }
                }
            }
        }

        (params, literals)
    }

    /// Asks the [`TypeInferencer`] for a hashmap of literal types, validating that they are all `Value` types.
    fn literal_types(&self) -> Result<Vec<(EqlTerm, &'ast ast::Value)>, EqlMapperError> {
        let (_, ste_vec_literals) = self.ste_vec_term_rhs_keys(&self.unifier.borrow());

        let literals = {
            let registry = self.registry.borrow();
            registry
                .get_nodes_and_types::<ast::Value>()
                .into_iter()
                .filter(|(node, _)| !matches!(node, ast::Value::Placeholder(_)))
        };

        let literal_nodes: Vec<(EqlTerm, &'ast ast::Value)> = literals
            .map(
                |(node, ty)| -> Result<Option<(EqlTerm, &'ast ast::Value)>, TypeError> {
                    let ty = ty.follow_tvars(&self.unifier.borrow());
                    if let Type::Value(Value::Eql(eql_term)) = &*ty {
                        let is_ste_vec_rhs = ste_vec_literals.contains(&NodeKey::new(node));
                        let eql_term = reclassify_eql_term_as_ste_vec_term_if(
                            eql_term.clone(),
                            is_ste_vec_rhs,
                        );
                        return Ok(Some((eql_term, node)));
                    }
                    Ok(None)
                },
            )
            .filter_map(Result::transpose)
            .collect::<Result<Vec<_>, _>>()?;

        Ok(literal_nodes)
    }

    fn node_types(&self) -> Result<HashMap<NodeKey<'ast>, Type>, EqlMapperError> {
        let registry = self.registry.borrow();
        let node_types = registry.node_types();

        let mut resolved_node_types: HashMap<NodeKey<'ast>, Type> = HashMap::new();
        for (key, ty) in node_types {
            let ty = ty.follow_tvars(&self.unifier.borrow());
            if !matches!(&*ty, Type::Value(_)) {
                return Err(EqlMapperError::InternalError(String::from(
                    "expected type to be resolved",
                )));
            }
            resolved_node_types.insert(key, (*ty).clone());
        }

        Ok(resolved_node_types)
    }
}

/// Reclassifies a resolved param [`Value`] as a STE-vec ordering term when it
/// is the right-hand side of a jsonb sv ordering comparison.
///
/// Only EQL `Partial` / `Full` values are reclassified; everything else is
/// returned unchanged.
fn reclassify_as_ste_vec_term_if(value: Value, is_ste_vec_rhs: bool) -> Value {
    if let Value::Eql(eql_term) = value {
        Value::Eql(reclassify_eql_term_as_ste_vec_term_if(
            eql_term,
            is_ste_vec_rhs,
        ))
    } else {
        value
    }
}

/// Reclassifies an [`EqlTerm`] as [`EqlTerm::SteVecTerm`] when `is_ste_vec_rhs`
/// is `true` and the term is a `Partial` or `Full` value.
fn reclassify_eql_term_as_ste_vec_term_if(eql_term: EqlTerm, is_ste_vec_rhs: bool) -> EqlTerm {
    if !is_ste_vec_rhs {
        return eql_term;
    }

    match eql_term {
        EqlTerm::Partial(eql_value, _) | EqlTerm::Full(eql_value) => EqlTerm::SteVecTerm(eql_value),
        other => other,
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
