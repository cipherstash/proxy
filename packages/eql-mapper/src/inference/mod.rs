mod infer_type;
mod infer_type_impls;
mod registry;
mod sequence;
mod sql_types;
mod type_error;

pub mod unifier;

use unifier::{Unifier, *};

use std::{cell::RefCell, fmt::Debug, marker::PhantomData, ops::ControlFlow, rc::Rc, sync::Arc};

use infer_type::InferType;
use sqltk::parser::ast::{
    Delete, Expr, Function, FunctionArgExpr, Ident, Insert, ObjectName, Query, Select, SelectItem,
    SetExpr, Statement, ValueWithSpan, Values, WindowSpec,
};
use sqltk::{into_control_flow, AsNodeKey, Break, Visitable, Visitor};

use crate::{
    JsonSelectorSource, JsonValueSelectors, Param, QueryOperands, ScopeError, ScopeTracker,
    TableResolver,
};

pub(crate) use registry::*;
pub(crate) use sequence::*;
pub(crate) use sql_types::*;
pub(crate) use type_error::*;

/// [`Visitor`] implementation that performs type inference on AST nodes.
///
/// Type inference is performed only on the following node types:
///
/// - [`Statement`]
/// - [`Query`]
/// - [`Insert`]
/// - [`Delete`]
/// - [`Expr`]
/// - [`SetExpr`]
/// - [`Select`]
/// - [`Vec<SelectItem>`]
/// - [`Function`]
/// - [`Values`]
/// - [`Value`]
/// - [`WindowSpec`]
#[derive(Debug)]
pub struct TypeInferencer<'ast> {
    /// A snapshot of the the database schema - used by `TypeInferencer`'s [`InferType`] impls.
    table_resolver: Arc<TableResolver>,

    // The lexical scope - for resolving projection columns & expanding wildcards.
    scope_tracker: Rc<RefCell<ScopeTracker<'ast>>>,

    /// Implements the type unification algorithm.
    unifier: Rc<RefCell<Unifier<'ast>>>,

    /// The fused JSON value selectors discovered while inferring `=`/`<>` over
    /// encrypted JSON field accesses. Unification records *types* per node; this
    /// records the one *relationship* the proxy needs — which operand supplies
    /// the path for which value ([`crate::JsonValueSelectors`]).
    json_value_selectors: RefCell<JsonValueSelectors<'ast>>,

    /// The operands that appear in a query position, so the proxy can project
    /// their payloads to query operands ([`crate::QueryOperands`]). Recorded
    /// here rather than derived later because it is a fact about the statement's
    /// shape, and the proxy needs it before it encrypts anything.
    query_operands: RefCell<QueryOperands<'ast>>,

    _ast: PhantomData<&'ast ()>,
}

impl<'ast> TypeInferencer<'ast> {
    /// Create a new `TypeInferencer`.
    pub fn new(
        table_resolver: impl Into<Arc<TableResolver>>,
        scope: impl Into<Rc<RefCell<ScopeTracker<'ast>>>>,
        unifier: impl Into<Rc<RefCell<Unifier<'ast>>>>,
    ) -> Self {
        Self {
            table_resolver: table_resolver.into(),
            scope_tracker: scope.into(),
            unifier: unifier.into(),
            json_value_selectors: RefCell::new(JsonValueSelectors::default()),
            query_operands: RefCell::new(QueryOperands::default()),
            _ast: PhantomData,
        }
    }

    /// Takes the fused JSON value selectors accumulated during inference,
    /// leaving the inferencer's set empty.
    pub(crate) fn take_json_value_selectors(&self) -> JsonValueSelectors<'ast> {
        std::mem::take(&mut self.json_value_selectors.borrow_mut())
    }

    /// Takes the recorded query operands, leaving the inferencer's set empty.
    pub(crate) fn take_query_operands(&self) -> QueryOperands<'ast> {
        std::mem::take(&mut self.query_operands.borrow_mut())
    }

    pub(crate) fn record_query_operand_param(&self, param: Param) {
        self.query_operands.borrow_mut().record_param(param);
    }

    pub(crate) fn record_query_operand_literal(&self, node: &'ast sqltk::parser::ast::Value) {
        self.query_operands.borrow_mut().record_literal(node);
    }

    pub(crate) fn record_json_value_selector_param(
        &self,
        param: Param,
        source: JsonSelectorSource,
    ) {
        self.json_value_selectors
            .borrow_mut()
            .record_param(param, source);
    }

    pub(crate) fn record_json_value_selector_literal(
        &self,
        node: &'ast sqltk::parser::ast::Value,
        source: JsonSelectorSource,
    ) {
        self.json_value_selectors
            .borrow_mut()
            .record_literal(node, source);
    }

    pub(crate) fn get_node_type<N: AsNodeKey>(&self, node: &'ast N) -> Arc<Type> {
        self.unifier.borrow_mut().get_node_type(node)
    }

    /// Requires `node` to have a type implementing `eql_trait`.
    ///
    /// A native type satisfies every bound trivially, so this only bites for an
    /// encrypted column, whose domain must carry the corresponding term.
    ///
    /// The node's *resolved* type is unified with the bound rather than the node
    /// merely being pointed at a bounded variable: an unresolved variable
    /// satisfies any bound vacuously, so binding alone defers the check
    /// indefinitely and never rejects the column.
    pub(crate) fn unify_node_with_bound<N: AsNodeKey>(
        &self,
        node: &'ast N,
        eql_trait: EqlTrait,
    ) -> Result<(), TypeError> {
        let bounded = self
            .unifier
            .borrow_mut()
            .fresh_bounded_tvar(eql_trait.into());
        let unified = self.unify(self.get_node_type(node), bounded)?;
        self.unify_node_with_type(node, unified)?;

        Ok(())
    }

    #[allow(unused)]
    pub(crate) fn peek_node_type<N: AsNodeKey>(&self, node: &'ast N) -> Option<Arc<Type>> {
        self.unifier.borrow_mut().peek_node_type(node)
    }

    pub(crate) fn get_param_type(&self, param: &'ast String) -> Arc<Type> {
        self.unifier.borrow_mut().get_param_type(param)
    }

    /// Tries to unify two types but does not record the result.
    /// Recording the result is up to the caller.
    #[must_use = "the result of unify must ultimately be associated with an AST node"]
    fn unify(
        &self,
        lhs: impl Into<Arc<Type>>,
        rhs: impl Into<Arc<Type>>,
    ) -> Result<Arc<Type>, TypeError> {
        self.unifier.borrow_mut().unify(lhs.into(), rhs.into())
    }

    /// Unifies the types of two nodes with a third type and records the unification result.
    /// After a successful unification both nodes will refer to the same type.
    fn unify_nodes_with_type<N1: AsNodeKey, N2: AsNodeKey>(
        &self,
        lhs: &'ast N1,
        rhs: &'ast N2,
        ty: impl Into<Arc<Type>>,
    ) -> Result<Arc<Type>, TypeError> {
        self.unify(
            ty,
            self.unify(self.get_node_type(lhs), self.get_node_type(rhs))?,
        )
    }

    /// Unifies the type of a node with a second type and records the unification result.
    fn unify_node_with_type<N: AsNodeKey>(
        &self,
        node: &'ast N,
        ty: impl Into<Arc<Type>>,
    ) -> Result<Arc<Type>, TypeError> {
        self.unify(self.get_node_type(node), ty)
    }

    /// Unifies the types of two nodes with each other.
    /// After a successful unification both nodes will refer to the same type.
    fn unify_nodes<N1: AsNodeKey + Debug, N2: AsNodeKey + Debug>(
        &self,
        lhs: &'ast N1,
        rhs: &'ast N2,
    ) -> Result<Arc<Type>, TypeError> {
        match self.unify(self.get_node_type(lhs), self.get_node_type(rhs)) {
            Ok(unified) => Ok(unified),
            Err(err) => Err(TypeError::OnNodes(
                format!("{lhs:?}"),
                self.get_node_type(lhs),
                format!("{rhs:?}"),
                self.get_node_type(rhs),
                err.to_string(),
            )),
        }
    }

    fn unify_all_with_type<N: Debug + AsNodeKey>(
        &self,
        nodes: &'ast [N],
        ty: impl Into<Arc<Type>>,
    ) -> Result<Arc<Type>, TypeError> {
        let unified = nodes
            .iter()
            .try_fold(ty.into(), |ty, node| self.unify_node_with_type(node, ty))?;

        Ok(unified)
    }

    fn fresh_tvar(&self) -> Arc<Type> {
        self.unifier.borrow_mut().fresh_tvar()
    }

    fn resolve_ident(&self, ident: &Ident) -> Result<Arc<Type>, ScopeError> {
        self.scope_tracker.borrow().resolve_ident(ident)
    }

    fn resolve_compound_ident(&self, idents: &[Ident]) -> Result<Arc<Type>, ScopeError> {
        self.scope_tracker.borrow().resolve_compound_ident(idents)
    }

    fn resolve_wildcard(&self) -> Result<Arc<Type>, ScopeError> {
        self.scope_tracker.borrow().resolve_wildcard()
    }

    fn resolve_qualified_wildcard(&self, idents: &ObjectName) -> Result<Arc<Type>, ScopeError> {
        self.scope_tracker
            .borrow()
            .resolve_qualified_wildcard(idents)
    }
}

macro_rules! dispatch {
    ($self:ident, $method:ident, $node:ident, $ty:ty) => {
        if let Some($node) = $node.downcast_ref::<$ty>() {
            into_control_flow($self.$method($node))?;
        }
    };
}

macro_rules! dispatch_all {
    ($self:ident, $method:ident, $node:ident) => {
        // Thought: as an optimisation this list should be order of most likely to be encountered. Expr & Value should
        // be tested for first.
        dispatch!($self, $method, $node, Statement);
        dispatch!($self, $method, $node, Query);
        dispatch!($self, $method, $node, Insert);
        dispatch!($self, $method, $node, Delete);
        dispatch!($self, $method, $node, Expr);
        dispatch!($self, $method, $node, SetExpr);
        dispatch!($self, $method, $node, Select);
        dispatch!($self, $method, $node, Vec<SelectItem>);
        dispatch!($self, $method, $node, SelectItem);
        dispatch!($self, $method, $node, Function);
        dispatch!($self, $method, $node, FunctionArgExpr);
        dispatch!($self, $method, $node, Values);
        dispatch!($self, $method, $node, ValueWithSpan);
        dispatch!($self, $method, $node, sqltk::parser::ast::Value);
        dispatch!($self, $method, $node, WindowSpec);
    };
}

/// # About this [`Visitor`] implementation.
///
/// On [`Visitor::enter`] invokes [`InferType::infer_enter`].
/// On [`Visitor::exit`] invokes [`InferType::infer_exit`].
impl<'ast> Visitor<'ast> for TypeInferencer<'ast> {
    type Error = TypeError;

    fn enter<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        dispatch_all!(self, infer_enter, node);
        ControlFlow::Continue(())
    }

    fn exit<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        dispatch_all!(self, infer_exit, node);
        ControlFlow::Continue(())
    }
}
