use super::importer::{ImportError, Importer};
use crate::{
    inference::{unifier, TypeError, TypeInferencer},
    unifier::{EqlValue, Unifier},
    DepMut, EqlColInProjectionAndGroupBy, FailOnPlaceholderChange, GroupByEqlCol,
    OrderByExprWithEqlType, PreserveEffectiveAliases, Projection, ReplacePlaintextEqlLiterals, ScopeError,
    ScopeTracker, TableResolver, TransformationRule, Type, TypeRegistry,
    UseEquivalentSqlFuncForEqlTypes, Value, ValueTracker,
};
use sqlparser::ast::{self as ast, Statement};
use sqltk::{AsNodeKey, Break, NodeKey, NodePath, Transform, Transformable, Visitable, Visitor};
use std::{
    cell::RefCell, collections::HashMap, marker::PhantomData, ops::ControlFlow, rc::Rc, sync::Arc,
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
            let node_types = mapper.node_types();

            if projection.is_err() || params.is_err() || literals.is_err() || node_types.is_err() {
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
                node_types: Arc::new(node_types?),
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

    pub node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
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
}

/// `EqlMapper` can safely convert a SQL statement into an equivalent statement where all of the plaintext literals have
/// been converted to EQL payloads containing the encrypted literal and/or encrypted representations of those literals.
#[derive(Debug)]
struct EqlMapper<'ast> {
    scope_tracker: Rc<RefCell<ScopeTracker<'ast>>>,
    importer: Rc<RefCell<Importer<'ast>>>,
    inferencer: Rc<RefCell<TypeInferencer<'ast>>>,
    registry: Rc<RefCell<TypeRegistry<'ast>>>,
    value_tracker: Rc<RefCell<ValueTracker<'ast>>>,
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
        let unifier = DepMut::new(Unifier::new(&registry));
        let value_tracker = DepMut::new(ValueTracker::new(&registry));

        let inferencer = DepMut::new(TypeInferencer::new(
            table_resolver.clone(),
            &scope_tracker,
            &registry,
            &unifier,
            &value_tracker,
        ));

        Self {
            scope_tracker: scope_tracker.into(),
            importer: importer.into(),
            inferencer: inferencer.into(),
            registry: registry.into(),
            value_tracker: value_tracker.into(),
            _ast: PhantomData,
        }
    }

    fn statement_type(
        &self,
        statement: &'ast Statement,
    ) -> Result<Option<Projection>, EqlMapperError> {
        let reg = self.registry.borrow_mut();

        match reg.get_type(statement) {
            Some(ty) => {
                let projection = ty.resolved_as::<crate::unifier::Projection>(&reg)?;
                Ok(Some(Projection::try_from(&*projection)?))
            }
            None => Err(EqlMapperError::InternalError(format!(
                "missing type for statement: {statement}"
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

    fn node_types(&self) -> Result<HashMap<NodeKey<'ast>, Type>, EqlMapperError> {
        let inferencer = self.inferencer.borrow();
        let node_types = inferencer.node_types();

        let mut resolved_node_types: HashMap<NodeKey<'ast>, Type> = HashMap::new();
        for (key, tcell) in node_types {
            resolved_node_types.insert(
                key,
                Type::try_from(&*tcell.resolved(&self.registry.borrow())?)?,
            );
        }

        Ok(resolved_node_types)
    }
}

impl<'ast> TypedStatement<'ast> {
    /// Transforms the SQL statement by replacing all plaintext literals with EQL equivalents.
    pub fn transform(
        &self,
        encrypted_literals: HashMap<NodeKey<'ast>, ast::Expr>,
    ) -> Result<Statement, EqlMapperError> {
        for (_, target) in self.literals.iter() {
            if !encrypted_literals.contains_key(&target.as_node_key()) {
                return Err(EqlMapperError::Transform(String::from("encrypted literals refers to a literal node which is not present in the SQL statement")));
            }
        }

        let mut transformer =
            EncryptedStatement::new(encrypted_literals, Arc::clone(&self.node_types));

        let statement = self.statement.apply_transform(&mut transformer)?;
        transformer.check_postcondition()?;
        Ok(statement)
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
}

#[derive(Debug)]
struct EncryptedStatement<'ast> {
    transformation_rules: (
        EqlColInProjectionAndGroupBy<'ast>,
        GroupByEqlCol<'ast>,
        OrderByExprWithEqlType<'ast>,
        PreserveEffectiveAliases,
        ReplacePlaintextEqlLiterals<'ast>,
        UseEquivalentSqlFuncForEqlTypes<'ast>,
        FailOnPlaceholderChange,
    ),
}

impl<'ast> EncryptedStatement<'ast> {
    fn new(
        encrypted_literals: HashMap<NodeKey<'ast>, ast::Expr>,
        node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
    ) -> Self {
        Self {
            transformation_rules: (
                EqlColInProjectionAndGroupBy::new(Arc::clone(&node_types)),
                GroupByEqlCol::new(Arc::clone(&node_types)),
                OrderByExprWithEqlType::new(Arc::clone(&node_types)),
                PreserveEffectiveAliases,
                ReplacePlaintextEqlLiterals::new(encrypted_literals),
                UseEquivalentSqlFuncForEqlTypes::new(Arc::clone(&node_types)),
                FailOnPlaceholderChange,
            ),
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
        node_path: &NodePath<'ast>,
        mut target_node: N,
    ) -> Result<N, Self::Error> {
        self.transformation_rules
            .apply(node_path, &mut target_node)?;

        Ok(target_node)
    }

    fn check_postcondition(&self) -> Result<(), Self::Error> {
        self.transformation_rules.check_postcondition()
    }
}

/// [`Visitor`] implementation that composes the [`ScopeTracker`] visitor, the [`Importer`] and the [`TypeInferencer`]
/// visitors.
impl<'ast> Visitor<'ast> for EqlMapper<'ast> {
    type Error = EqlMapperError;

    fn enter<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        self.value_tracker.borrow_mut().enter(node);

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

        self.value_tracker.borrow_mut().exit(node);

        ControlFlow::Continue(())
    }
}
