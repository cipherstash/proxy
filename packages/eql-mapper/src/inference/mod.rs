mod infer_type;
mod infer_type_impls;
mod registry;
mod sequence;
mod type_error;

pub mod unifier;

use unifier::{Unifier, *};

use std::{
    cell::RefCell, fmt::Debug, marker::PhantomData,
    ops::ControlFlow, rc::Rc, sync::Arc,
};

use infer_type::InferType;
use sqlparser::ast::{
    Delete, Expr, Function, Ident, Insert, Query, Select, SelectItem, SetExpr, Statement, Values,
};
use sqltk::{into_control_flow, AsNodeKey, Break, Visitable, Visitor};

use crate::{ScopeError, ScopeTracker, TableResolver};

pub(crate) use registry::*;
pub(crate) use sequence::*;
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
#[derive(Debug)]
pub struct TypeInferencer<'ast> {
    /// A snapshot of the the database schema - used by `TypeInferencer`'s [`InferType`] impls.
    table_resolver: Arc<TableResolver>,

    // The lexical scope - for resolving projection columns & expanding wildcards.
    scope_tracker: Rc<RefCell<ScopeTracker<'ast>>>,

    /// Implements the type unification algorithm.
    unifier: Rc<RefCell<Unifier<'ast>>>,

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
            _ast: PhantomData,
        }
    }

    pub(crate) fn get_node_type<N: AsNodeKey>(&self, node: &'ast N) -> Arc<Type> {
        self.unifier.borrow_mut().get_node_type(node)
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
                Box::new(err),
                format!("{:?}", lhs),
                self.get_node_type(lhs),
                format!("{:?}", rhs),
                self.get_node_type(rhs),
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

    fn resolve_qualified_wildcard(&self, idents: &[Ident]) -> Result<Arc<Type>, ScopeError> {
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
        dispatch!($self, $method, $node, Values);
        dispatch!($self, $method, $node, sqlparser::ast::Value);
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
