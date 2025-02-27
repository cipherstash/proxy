use super::importer::{ImportError, Importer};
use crate::{
    eql_function_tracker::{EqlFunctionTracker, EqlFunctionTrackerError},
    inference::{unifier, TypeError, TypeInferencer},
    unifier::{EqlValue, Unifier},
    DepMut, NodeKey, Projection, ProjectionColumn, ScopeError, ScopeTracker, TableResolver,
    TypeRegistry, Value,
};
use sqlparser::{
    ast::{self as ast, Statement},
    tokenizer::Span,
};
use sqltk::{convert_control_flow, Break, Transform, Transformable, Visitable, Visitor};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    marker::PhantomData,
    ops::ControlFlow,
    rc::Rc,
    sync::Arc,
};

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
    schema: Arc<TableResolver>,
    statement: &'ast Statement,
) -> Result<TypedStatement<'ast>, EqlMapperError> {
    let mut mapper = EqlMapper::<'ast>::new_from_schema(schema);
    match statement.accept(&mut mapper) {
        ControlFlow::Continue(()) => {
            let projection = mapper.statement_type(statement);
            let params = mapper.param_types();
            let literals = mapper.literal_types();

            if projection.is_err() || params.is_err() || literals.is_err() {
                #[cfg(test)]
                {
                    mapper.inferencer.borrow().dump_registry(statement);
                }
            }

            Ok(TypedStatement {
                statement,
                projection: projection?,
                params: params?,
                literals: literals?,
                nodes_to_wrap: mapper.nodes_to_wrap(),
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
    pub projection: Option<Projection>,

    /// The types of all params discovered from [`Value::Placeholder`] nodes in the SQL statement.
    pub params: Vec<Value>,

    /// The types and values of all literals from the SQL statement.
    pub literals: Vec<(EqlValue, &'ast ast::Expr)>,

    pub nodes_to_wrap: HashSet<NodeKey<'ast>>,
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

    #[error(transparent)]
    EqlFunctionTracker(#[from] EqlFunctionTrackerError),
}

/// `EqlMapper` can safely convert a SQL statement into an equivalent statement where all of the plaintext literals have
/// been converted to EQL payloads containing the encrypted literal and/or encrypted representations of those literals.
#[derive(Debug)]
struct EqlMapper<'ast> {
    scope_tracker: Rc<RefCell<ScopeTracker<'ast>>>,
    importer: Rc<RefCell<Importer<'ast>>>,
    inferencer: Rc<RefCell<TypeInferencer<'ast>>>,
    registry: Rc<RefCell<TypeRegistry<'ast>>>,
    eql_function_tracker: Rc<RefCell<EqlFunctionTracker<'ast>>>,
    _ast: PhantomData<&'ast ()>,
}

impl<'ast> EqlMapper<'ast> {
    /// Build an `EqlMapper`, initialising all the other visitor implementations that it depends on.
    fn new_from_schema(table_resolver: Arc<TableResolver>) -> Self {
        let scope_tracker = DepMut::new(ScopeTracker::new());
        let registry = DepMut::new(TypeRegistry::new());
        let importer = DepMut::new(Importer::new(
            table_resolver.clone(),
            &registry,
            &scope_tracker,
        ));
        let eql_function_tracker = DepMut::new(EqlFunctionTracker::new(&registry));
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
            eql_function_tracker: eql_function_tracker.into(),
            _ast: PhantomData,
        }
    }

    /// Asks the [`TypeInferencer`] for a hashmap of node types.
    fn statement_type(
        &self,
        statement: &'ast Statement,
    ) -> Result<Option<Projection>, EqlMapperError> {
        let reg = self.registry.borrow();
        match reg.get_type_by_node_key(&NodeKey::new(statement)) {
            Some(ty_cell) => match ty_cell.resolved(&reg) {
                Ok(ty_ref) => match &*ty_ref {
                    unifier::Type::Constructor(unifier::Constructor::Projection(
                        unifier::Projection::WithColumns(cols),
                    )) => {
                        let cols = cols.flatten();
                        Ok(Some(Projection::WithColumns(
                            cols.0
                                .iter()
                                .map(|col| match &*col.ty.as_type() {
                                    unifier::Type::Constructor(unifier::Constructor::Value(
                                        value,
                                    )) => Ok(ProjectionColumn {
                                        ty: value.try_into()?,
                                        alias: col.alias.clone(),
                                    }),
                                    ty => Err(EqlMapperError::InternalError(format!(
                                        "unexpected type {} in projection column",
                                        ty
                                    ))),
                                })
                                .collect::<Result<Vec<_>, _>>()?,
                        )))
                    }
                    unifier::Type::Constructor(unifier::Constructor::Projection(
                        unifier::Projection::Empty,
                    )) => Ok(Some(Projection::Empty)),
                    unexpected => Err(EqlMapperError::InternalError(format!(
                        "unexpected type {unexpected} for statement"
                    ))),
                },
                Err(err) => Err(EqlMapperError::from(err)),
            },
            None => Err(EqlMapperError::InternalError(
                "could not find statement type information".to_string(),
            )),
        }
    }

    /// Asks the [`TypeInferencer`] for a hashmap of parameter types.
    fn param_types(&self) -> Result<Vec<Value>, EqlMapperError> {
        let param_types = self.inferencer.borrow().param_types()?;

        let mut param_types: Vec<(i32, Value)> = param_types
            .iter()
            .map(|(param, ty)| {
                Value::try_from(ty).and_then(|ty| {
                    param
                        .replace("$", "")
                        .parse()
                        .map(|idx| (idx, ty))
                        .map_err(|_| {
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

    /// Asks the [`TypeInferencer`] for a hashmap of literal types, validating that they are all `Scalar` types.
    fn literal_types(&self) -> Result<Vec<(EqlValue, &'ast ast::Expr)>, EqlMapperError> {
        let inferencer = self.inferencer.borrow();
        let literal_nodes = inferencer.literal_nodes();

        let literals: Vec<(EqlValue, &'ast ast::Expr)> = literal_nodes
            .iter()
            .map(|node_key| match inferencer.get_type_by_node_key(node_key) {
                Some(ty) => {
                    if let unifier::Type::Constructor(unifier::Constructor::Value(
                        eql_ty @ unifier::Value::Eql(_),
                    )) = &*ty.resolved(&self.registry.borrow())?
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

    /// Takes `eql_function_tracker`, consumes it, and returns a `HashSet` of keys for nodes
    /// that the type checker has marked for wrapping with EQL function calls.
    fn nodes_to_wrap(&self) -> HashSet<NodeKey<'ast>> {
        self.eql_function_tracker.take().into_to_wrap()
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

        self.statement.apply_transform(&mut EncryptedStatement::new(
            encrypted_literals,
            &self.nodes_to_wrap,
        ))
    }

    pub fn literal_values(&self) -> Vec<&sqlparser::ast::Value> {
        if self.literals.is_empty() {
            return vec![];
        }

        self.literals
            .iter()
            .map(|(_eql_value, expr)| {
                if let sqlparser::ast::Expr::Value(value) = expr {
                    value
                } else {
                    &sqlparser::ast::Value::Null
                }
            })
            .collect::<Vec<_>>()
    }

    /// Returns `true` if the typed statement has nodes that the type checker has marked for wrapping with EQL function calls.
    pub fn has_nodes_to_wrap(&self) -> bool {
        !self.nodes_to_wrap.is_empty()
    }
}

#[derive(Debug)]
struct EncryptedStatement<'ast> {
    encrypted_literals: HashMap<&'ast ast::Expr, ast::Expr>,
    nodes_to_wrap: &'ast HashSet<NodeKey<'ast>>,
}

impl<'ast> EncryptedStatement<'ast> {
    fn new(
        encrypted_literals: HashMap<&'ast ast::Expr, ast::Expr>,
        nodes_to_wrap: &'ast HashSet<NodeKey<'ast>>,
    ) -> Self {
        Self {
            encrypted_literals,
            nodes_to_wrap,
        }
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
                    ast::Expr::Value(ast::Value::Placeholder(_))
                        if original_value != target_value =>
                    {
                        return Err(EqlMapperError::InternalError(
                            "attempt was made to update placeholder with literal".to_string(),
                        ));
                    }

                    // Wrap identifiers (e.g. `encrypted_col`) and compound identifiers (e.g. `some_tbl.encrypted_col`)
                    // in an EQL function if the type checker has marked them as nodes that need to be
                    // wrapped.
                    //
                    // For example (assuming that `encrypted_col` is an identifier for an EQL column) transform
                    // `encrypted_col` to `cs_ore_64_8_v1(encrypted_col)`.
                    ast::Expr::Identifier(_) | ast::Expr::CompoundIdentifier(_) => {
                        let node_key = NodeKey::new(original_value);

                        if self.nodes_to_wrap.contains(&node_key) {
                            *target_value =
                                make_eql_function_node("cs_ore_64_8_v1", original_value.clone());
                        }
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

fn make_eql_function_node(function_name: &str, arg: ast::Expr) -> ast::Expr {
    ast::Expr::Function(ast::Function {
        uses_odbc_syntax: false,
        parameters: ast::FunctionArguments::None,
        filter: None,
        null_treatment: None,
        over: None,
        within_group: vec![],
        name: ast::ObjectName(vec![ast::Ident {
            value: function_name.to_string(),
            quote_style: None,
            span: Span::empty(),
        }]),
        args: ast::FunctionArguments::List(ast::FunctionArgumentList {
            duplicate_treatment: None,
            clauses: vec![],
            args: vec![ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(arg))],
        }),
    })
}

/// [`Visitor`] implememtation that composes the [`Scope`] visitor, the [`Importer`] and the [`TypeInferencer`]
/// visitors.
impl<'ast> Visitor<'ast> for EqlMapper<'ast> {
    type Error = EqlMapperError;

    fn enter<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        convert_control_flow(self.scope_tracker.borrow_mut().enter(node))?;
        convert_control_flow(self.importer.borrow_mut().enter(node))?;
        convert_control_flow(self.eql_function_tracker.borrow_mut().enter(node))?;
        convert_control_flow(self.inferencer.borrow_mut().enter(node))?;

        ControlFlow::Continue(())
    }

    fn exit<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        convert_control_flow(self.inferencer.borrow_mut().exit(node))?;
        convert_control_flow(self.eql_function_tracker.borrow_mut().exit(node))?;
        convert_control_flow(self.importer.borrow_mut().exit(node))?;
        convert_control_flow(self.scope_tracker.borrow_mut().exit(node))?;

        ControlFlow::Continue(())
    }
}
