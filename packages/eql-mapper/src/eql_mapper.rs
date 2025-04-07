use super::importer::{ImportError, Importer};
use crate::{
    eql_function_tracker::{EqlFunctionTracker, EqlFunctionTrackerError},
    inference::{unifier, TypeError, TypeInferencer},
    unifier::{EqlValue, Unifier},
    DepMut, EndsWith, Projection, ScopeError, ScopeTracker, TableResolver, Type,
    TypeRegistry, Value,
};
use sqlparser::ast::{
    self as ast, Expr, Function, FunctionArg, FunctionArgumentList, FunctionArguments, GroupByExpr,
    Ident, ObjectName, Select, SelectItem, Statement,
};
use sqltk::{AsNodeKey, Break, Context, NodeKey, Transform, Transformable, Visitable, Visitor};
use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    convert::Infallible,
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
            let node_types = mapper.node_types();

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
                node_types: node_types?,
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

    pub node_types: HashMap<NodeKey<'ast>, Type>,
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

    #[error(transparent)]
    EqlFunctionTracker(#[from] EqlFunctionTrackerError),
}

/// `EqlMapper` can safely convert a SQL statement into an equivalent statement where all of the plaintext literals have
/// been converted to EQL payloads containing the encrypted literal and/or encrypted representations of those literals.
#[derive(Debug)]
struct EqlMapper<'ast> {
    scope_tracker: Rc<RefCell<ScopeTracker>>,
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

    fn statement_type(
        &self,
        statement: &'ast Statement,
    ) -> Result<Option<Projection>, EqlMapperError> {
        let reg = self.registry.borrow_mut();

        match reg.get_type(statement) {
            Some(ty) => {
                let projection = ty.resolved_as::<crate::unifier::Projection>(&*reg)?;
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
                Type::try_from(&*tcell.resolved(&*self.registry.borrow())?)?,
            );
        }

        Ok(resolved_node_types)
    }

    /// Takes `eql_function_tracker`, consumes it, and returns a `HashSet` of keys for nodes
    /// that the type checker has marked for wrapping with EQL function calls.
    fn nodes_to_wrap(&self) -> HashSet<NodeKey<'ast>> {
        self.eql_function_tracker.take().into_to_wrap()
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

        self.statement.apply_transform(&mut EncryptedStatement::new(
            encrypted_literals,
            &self.nodes_to_wrap,
            &self.node_types,
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
    encrypted_literals: HashMap<NodeKey<'ast>, ast::Expr>,
    nodes_to_wrap: &'ast HashSet<NodeKey<'ast>>,
    node_types: &'ast HashMap<NodeKey<'ast>, Type>,
}

impl<'ast> EncryptedStatement<'ast> {
    fn new(
        encrypted_literals: HashMap<NodeKey<'ast>, ast::Expr>,
        nodes_to_wrap: &'ast HashSet<NodeKey<'ast>>,
        node_types: &'ast HashMap<NodeKey<'ast>, Type>,
    ) -> Self {
        Self {
            encrypted_literals,
            nodes_to_wrap,
            node_types,
        }
    }

    /// We need to know if an [`Expr`] using an EQL column in a projection has to be aggregated, or have its
    /// aggregation rewritten.
    ///
    /// # Scenario 1: The "grouping by the encrypted column" case
    ///
    /// In this case the encrypted column is mentioned in the GROUP BY clause so the grouped value must be extracted.
    ///
    /// ## Input
    ///
    /// ```sql
    /// SELECT
    ///   email_encrypted,
    ///   count(*)
    /// FROM orders
    /// GROUP BY email_encrypted;
    /// ```
    ///
    /// ## Output
    ///
    /// SELECT
    ///   cs_grouped_value_v1(email_encrypted) as email_encrypted,
    ///   count(*)
    /// FROM orders
    /// GROUP BY cs_ore_64_8_1(email_encrypted);
    ///
    /// # Scenario 2: The "grouping by another column" case
    ///
    /// In this case the encrypted column is NOT mentioned in the GROUP BY which means it already must be aggregated but
    /// we need to change the aggregate function: `MIN` becomes `cs_min_v1` and `MAX` becomes `cs_max_v1`.
    ///
    /// ## Input
    ///
    /// ```sql
    /// SELECT
    ///   MIN(email_encrypted) as email_encrypted,
    ///   completed_at
    /// FROM orders
    /// GROUP BY completed_at;
    /// ```
    ///
    /// ## Output
    ///
    /// ```sql
    /// SELECT
    ///   cs_min_v1(email_encrypted) as email_encrypted,
    ///   completed_at
    /// FROM orders
    /// GROUP BY completed_at;
    /// ```
    fn transform_projection_exprs_as_per_group_by_clause<N: Visitable>(
        &mut self,
        new_node: &mut N,
        original_node: &'ast N,
        context: &Context<'ast>,
    ) -> Result<(), EqlMapperError> {
        // Scenario 1: Wrap column expr in cs_grouped_value_1
        if let Some((select, _projection, _select_item, expr)) =
            EndsWith::<(&Select, &Vec<SelectItem>, &SelectItem, &mut Expr)>::try_match(
                context, new_node,
            )
        {
            if self.is_used_in_group_by_clause(&select.group_by, original_node) {
                *expr = self.wrap_in_single_arg_function(
                    expr.clone(),
                    ObjectName(vec![Ident::new("cs_grouped_value_v1")]),
                );
            }
        }

        // Scenario 1: add alias to column in order to preserve the effective alias prior to transformation.
        if let Some((select, _projection, select_item)) =
            EndsWith::<(&Select, &Vec<SelectItem>, &mut SelectItem)>::try_match(context, new_node)
        {
            if self.is_used_in_group_by_clause(&select.group_by, original_node) {
                *select_item = self.add_alias_to_unnamed_select_item(
                    select_item.clone(),
                    original_node.downcast_ref().unwrap(),
                );
            }
        }

        // Scenario 1: wrap the specific group clause expression in `cs_ore_64_8_v1(..)`
        if let Some((_group_by_clause, _exprs, expr)) =
            EndsWith::<(&GroupByExpr, &Vec<Expr>, &mut Expr)>::try_match(context, new_node)
        {
            let _s = format!(
                "TY: {:#?}",
                self.node_types.get(&original_node.as_node_key())
            );
            if let Some(Type::Value(Value::Eql(_))) =
                self.node_types.get(&original_node.as_node_key())
            {
                *expr = self.wrap_in_single_arg_function(
                    expr.clone(),
                    ObjectName(vec![Ident::new("cs_ore_64_8_v1")]),
                );
            }
        }

        Ok(())
    }

    fn wrap_in_single_arg_function(&self, to_wrap: Expr, name: ObjectName) -> Expr {
        Expr::Function(Function {
            name,
            parameters: FunctionArguments::None,
            args: FunctionArguments::List(FunctionArgumentList {
                args: vec![FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(
                    to_wrap.clone(),
                ))],
                duplicate_treatment: None,
                clauses: vec![],
            }),
            filter: None,
            null_treatment: None,
            over: None,
            within_group: vec![],
        })
    }

    /// Converts [`SelectItem::UnnamedExpr`] to [`SelectItem::ExprWithAlias`].
    ///
    /// All other variants are returned unmodified.
    fn add_alias_to_unnamed_select_item(
        &self,
        to_wrap: SelectItem,
        original_node: &SelectItem,
    ) -> SelectItem {
        match to_wrap {
            SelectItem::UnnamedExpr(expr) => {
                match self.derive_alias_from_select_item(original_node) {
                    Some(alias) => SelectItem::ExprWithAlias { expr, alias },
                    None => SelectItem::UnnamedExpr(expr),
                }
            }
            other => other,
        }
    }

    fn derive_alias_from_select_item(&self, node: &SelectItem) -> Option<Ident> {
        match node {
            SelectItem::UnnamedExpr(expr) => {
                /// Unwrap an [`Expr`] until we find a `Expr::Identifier`, `Expr::CompoundIdentifier` or `Expr::Function`.
                /// This is meant to emulate what Postgres does when it tries to derive a column name from an expression that
                /// has no alias.
                struct DeriveNameFromExpr {
                    found: Option<Ident>,
                }

                impl<'ast> Visitor<'ast> for DeriveNameFromExpr {
                    type Error = Infallible;

                    fn enter<N: Visitable>(
                        &mut self,
                        node: &'ast N,
                    ) -> ControlFlow<Break<Self::Error>> {
                        if let Some(expr) = node.downcast_ref::<Expr>() {
                            match expr {
                                Expr::Identifier(ident) => {
                                    self.found = Some(ident.clone());
                                    return ControlFlow::Break(Break::Finished);
                                }
                                Expr::CompoundIdentifier(obj_name) => {
                                    self.found = Some(obj_name.last().unwrap().clone());
                                    return ControlFlow::Break(Break::Finished);
                                }
                                Expr::Function(Function { name, .. }) => {
                                    self.found = Some(name.0.last().unwrap().clone());
                                    return ControlFlow::Break(Break::Finished);
                                }
                                _ => {}
                            }
                        }

                        ControlFlow::Continue(())
                    }
                }

                let mut visitor = DeriveNameFromExpr { found: None };
                expr.accept(&mut visitor);
                visitor.found
            }
            SelectItem::ExprWithAlias { expr: _, alias } => Some(alias.clone()),
            _ => None,
        }
    }

    /// Checks if `node` has an EQL type (encrypted) and that type is referenced in the `GROUP BY` clause of `select`.
    fn is_used_in_group_by_clause<N: AsNodeKey>(
        &self,
        group_by: &'ast GroupByExpr,
        node: &'ast N,
    ) -> bool {
        struct ContainsExprWithType<'ast, 't> {
            node_types: &'t HashMap<NodeKey<'ast>, Type>,
            needle: &'t Type,
            found: bool,
        }

        impl<'t, 'ast> Visitor<'ast> for ContainsExprWithType<'t, 'ast> {
            type Error = Infallible;

            fn enter<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
                if let Some(expr) = node.downcast_ref::<Expr>() {
                    if let Some(expr_ty) = self.node_types.get(&expr.as_node_key()) {
                        if expr_ty == self.needle {
                            self.found = true;
                            return ControlFlow::Break(Break::Finished);
                        }
                    }
                }

                ControlFlow::Continue(())
            }
        }

        match self.node_types.get(&node.as_node_key()) {
            Some(needle @ Type::Value(Value::Eql(_))) => match group_by {
                GroupByExpr::All(_) => true,
                GroupByExpr::Expressions(exprs, _) => {
                    let mut visitor = ContainsExprWithType {
                        node_types: &self.node_types,
                        needle,
                        found: false,
                    };
                    exprs.accept(&mut visitor);
                    return visitor.found;
                }
            },
            _ => false,
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
        mut new_node: N,
        original_node: &'ast N,
        context: &Context<'ast>,
    ) -> Result<N, Self::Error> {
        self.transform_projection_exprs_as_per_group_by_clause(
            &mut new_node,
            original_node,
            context,
        )?;

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

                    ast::Expr::Value(_) => {
                        if let Some(replacement) = self
                            .encrypted_literals
                            .remove(&original_value.as_node_key())
                        {
                            *target_value = replacement;
                        }
                    }

                    // Wrap identifiers (e.g. `encrypted_col`) and compound identifiers (e.g. `some_tbl.encrypted_col`)
                    // in an EQL function if the type checker has marked them as nodes that need to be
                    // wrapped.
                    //
                    // For example (assuming that `encrypted_col` is an identifier for an EQL column) transform
                    // `encrypted_col` to `cs_ore_64_8_v1(encrypted_col)`.
                    ast::Expr::Identifier(_) | ast::Expr::CompoundIdentifier(_) => {
                        let node_key = original_value.as_node_key();

                        if self.nodes_to_wrap.contains(&node_key) {
                            *target_value =
                                make_eql_function_node("cs_ore_64_8_v1", original_value.clone());
                        }
                    }

                    _ => { /* other variants are a no-op and don't require transformation */ }
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
        parameters: FunctionArguments::None,
        filter: None,
        null_treatment: None,
        over: None,
        within_group: vec![],
        name: ast::ObjectName(vec![ast::Ident {
            value: function_name.to_string(),
            quote_style: None,
        }]),
        args: FunctionArguments::List(ast::FunctionArgumentList {
            duplicate_treatment: None,
            clauses: vec![],
            args: vec![ast::FunctionArg::Unnamed(ast::FunctionArgExpr::Expr(arg))],
        }),
    })
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
        self.eql_function_tracker
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
        self.eql_function_tracker
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
