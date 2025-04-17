mod infer_type;
mod infer_type_impls;
mod registry;
mod sequence;
mod type_error;

pub mod unifier;

use tracing::{span, Level};
use unifier::{Unifier, *};

use std::{
    any::type_name, cell::RefCell, collections::HashMap, fmt::Debug, marker::PhantomData, ops::ControlFlow, rc::Rc, sync::Arc
};

use infer_type::InferType;
use sqlparser::ast::{
    Delete, Expr, Function, Insert, Query, Select, SelectItem, SetExpr, Statement, Values,
};
use sqltk::{into_control_flow, AsNodeKey, Break, NodeKey, Visitable, Visitor};

use crate::{Fmt, Param, ParamError, ScopeTracker, TableResolver};

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
pub struct TypeInferencer<'ast> {
    /// A snapshot of the the database schema - used by `TypeInferencer`'s [`InferType`] impls.
    table_resolver: Arc<TableResolver>,

    // The lexical scope - for resolving projection columns & expanding wildcards.
    scope_tracker: Rc<RefCell<ScopeTracker<'ast>>>,

    /// Associates types with AST nodes.
    reg: Rc<RefCell<TypeRegistry<'ast>>>,

    /// Implements the type unification algorithm.
    unifier: Rc<RefCell<Unifier<'ast>>>,

    _ast: PhantomData<&'ast ()>,
}

impl<'ast> TypeInferencer<'ast> {
    /// Create a new `TypeInferencer`.
    pub fn new(
        table_resolver: impl Into<Arc<TableResolver>>,
        scope: impl Into<Rc<RefCell<ScopeTracker<'ast>>>>,
        reg: impl Into<Rc<RefCell<TypeRegistry<'ast>>>>,
        unifier: impl Into<Rc<RefCell<Unifier<'ast>>>>,
    ) -> Self {
        Self {
            table_resolver: table_resolver.into(),
            scope_tracker: scope.into(),
            reg: reg.into(),
            unifier: unifier.into(),
            _ast: PhantomData,
        }
    }

    pub(crate) fn get_node_type<N: AsNodeKey>(&self, node: &'ast N) -> Arc<Type> {
        self.reg.borrow_mut().get_node_type(node)
    }

    pub(crate) fn get_node_type_var<N: AsNodeKey>(&self, node: &'ast N) -> TypeVar {
        self.reg.borrow_mut().get_node_type_var(node)
    }

    pub(crate) fn param_types(&self) -> Result<Vec<(Param, Arc<Type>)>, ParamError> {
        let mut params = self
            .reg
            .borrow()
            .get_params()
            .iter()
            .map(|(p, ty)| Param::try_from(*p).map(|p| (p, ty.clone())))
            .collect::<Result<Vec<_>, _>>()?;

        params.sort_by(|(a, _), (b, _)| a.cmp(b));

        Ok(params)
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

    fn substitute(&self, tvar: TypeVar, sub_ty: impl Into<Arc<Type>>) -> Arc<Type> {
        self.unifier.borrow_mut().substitute(tvar, sub_ty)
    }

    /// Unifies the types of two nodes with a third type and records the unification result.
    /// After a successful unification both nodes will refer to the same type.
    fn unify_nodes_with_type<N1: AsNodeKey, N2: AsNodeKey>(
        &self,
        lhs: &'ast N1,
        rhs: &'ast N2,
        ty: impl Into<Arc<Type>>,
    ) -> Result<Arc<Type>, TypeError> {
        let lhs_tvar = self.get_node_type_var(lhs);
        let rhs_tvar = self.get_node_type_var(rhs);
        let unified = self.unify(self.get_node_type(lhs), self.get_node_type(rhs))?;
        let unified = self.unify(unified, ty)?;
        let unified = self.substitute(lhs_tvar, unified);
        let unified = self.substitute(rhs_tvar, unified);
        Ok(unified)
    }

    /// Unifies the type of a node with a second type and records the unification result.
    fn unify_node_with_type<N: AsNodeKey>(
        &self,
        node: &'ast N,
        ty: impl Into<Arc<Type>>,
    ) -> Result<Arc<Type>, TypeError> {
        let node_tvar = self.get_node_type_var(node);
        let node_ty = self.get_node_type(node);
        let unified = self.unify(node_ty, ty)?;
        let unified = self.substitute(node_tvar, unified);
        Ok(unified)
    }

    /// Unifies the types of two nodes with each other.
    /// After a successful unification both nodes will refer to the same type.
    fn unify_nodes<N1: AsNodeKey + Debug, N2: AsNodeKey + Debug>(
        &self,
        lhs: &'ast N1,
        rhs: &'ast N2,
    ) -> Result<Arc<Type>, TypeError> {
        let lhs_tvar = self.get_node_type_var(lhs);
        let rhs_tvar = self.get_node_type_var(rhs);
        match self.unify(self.get_node_type(lhs), self.get_node_type(rhs)) {
            Ok(unified) => {
                let unified = self.substitute(lhs_tvar, unified);
                let unified = self.substitute(rhs_tvar, unified);
                Ok(unified)
            }
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

    pub(crate) fn fresh_tvar(&self) -> TypeVar {
        self.reg.borrow_mut().fresh_tvar()
    }

    pub(crate) fn node_types(&self) -> HashMap<NodeKey<'ast>, Option<Arc<Type>>> {
        self.reg.borrow().node_types()
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
        let span_outer = span!(
            Level::TRACE,
            "node",
            node_ty = type_name::<N>(),
            node = %Fmt(&NodeKey::new(node)),
        );
        let _guard_outer = span_outer.enter();

        dispatch_all!(self, infer_enter, node);

        let span_inner = span!(
            parent: &span_outer,
            Level::TRACE,
            "result",
            inferred = %self.get_node_type(node),
        );
        let _guard_inner = span_inner.enter();

        ControlFlow::Continue(())
    }

    fn exit<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        let span_outer = span!(
            Level::TRACE,
            "node",
            node_ty = type_name::<N>(),
            node = %Fmt(&NodeKey::new(node)),
        );

        let _guard_outer = span_outer.enter();

        dispatch_all!(self, infer_exit, node);

        let span_inner = span!(
            parent: &span_outer,
            Level::TRACE,
            "result",
            inferred = %self.get_node_type(node),
        );
        let _guard_inner = span_inner.enter();

        ControlFlow::Continue(())
    }
}

pub(crate) mod test_util {
    use sqltk::Visitable;

    use super::TypeInferencer;

    impl TypeInferencer<'_> {
        /// Dump all nodes from the registry to STDERR. Useful for debugging failing tests.
        pub(crate) fn dump_registry<N: Visitable>(&self, root_node: &N) {
            self.reg.borrow().dump_all_nodes(root_node);
        }
    }
}
