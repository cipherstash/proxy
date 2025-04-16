mod infer_type;
mod infer_type_impls;
mod registry;
mod sequence;
mod type_error;

pub mod unifier;

use unifier::{Unifier, *};

use std::{
    cell::RefCell, collections::HashMap, fmt::Debug, marker::PhantomData, ops::ControlFlow, rc::Rc,
    sync::Arc,
};

use infer_type::InferType;
use sqlparser::ast::{
    Delete, Expr, Function, Insert, Query, Select, SelectItem, SetExpr, Statement, Values,
};
use sqltk::{into_control_flow, AsNodeKey, Break, NodeKey, Visitable, Visitor};

use crate::{Param, ParamError, ScopeTracker, TableResolver};

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

    /// Shorthand for calling `self.reg.borrow_mut().get_type(node)` in [`InferType`] implementations for `TypeInferencer`.
    pub(crate) fn get_type_var_of_node<N: AsNodeKey>(&self, node: &'ast N) -> TypeVar {
        self.reg.borrow_mut().get_or_init_type(node)
    }

    pub(crate) fn get_type_of_node<N: AsNodeKey>(&self, node: &'ast N) -> Arc<Type> {
        let tvar = self.get_type_var_of_node(node);
        self.reg.borrow().get_type_by_tvar(tvar).unwrap_or(Arc::new(Type::Var(tvar)))
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

    /// Unifies the types of two nodes with a third type and records the unification result.
    /// After a successful unification both nodes will refer to the same type.
    fn unify_nodes_with_type<N1: AsNodeKey, N2: AsNodeKey>(
        &self,
        lhs: &'ast N1,
        rhs: &'ast N2,
        ty: impl Into<Arc<Type>>,
    ) -> Result<Arc<Type>, TypeError> {
        let unifier = &mut *self.unifier.borrow_mut();
        let lhs_tvar = self.get_type_var_of_node(lhs);
        let rhs_tvar = self.get_type_var_of_node(rhs);
        let reg = &mut *self.reg.borrow_mut();
        let unified = unifier.unify(self.get_type_of_node(lhs), self.get_type_of_node(rhs))?;
        let unified = unifier.unify(unified, ty)?;
        let unified = reg.substitute(lhs_tvar, unified);
        let unified = reg.substitute(rhs_tvar, unified);
        Ok(unified)
    }

    /// Unifies the type of a node with a second type and records the unification result.
    fn unify_node_with_type<N: AsNodeKey>(
        &self,
        node: &'ast N,
        ty: impl Into<Arc<Type>>,
    ) -> Result<Arc<Type>, TypeError> {
        let unifier = &mut *self.unifier.borrow_mut();
        let node_tvar = self.get_type_var_of_node(node);
        let node_ty = self.get_type_of_node(node);
        let reg = &mut *self.reg.borrow_mut();
        let unified = unifier.unify(node_ty, ty)?;
        let unified = reg.substitute(node_tvar, unified);
        Ok(unified)
    }

    /// Unifies the types of two nodes with each other.
    /// After a successful unification both nodes will refer to the same type.
    fn unify_nodes<N1: AsNodeKey + Debug, N2: AsNodeKey + Debug>(
        &self,
        lhs: &'ast N1,
        rhs: &'ast N2,
    ) -> Result<Arc<Type>, TypeError> {
        let unifier = &mut *self.unifier.borrow_mut();
        let lhs_tvar = self.get_type_var_of_node(lhs);
        let rhs_tvar = self.get_type_var_of_node(rhs);
        let reg = &mut *self.reg.borrow_mut();
        match unifier.unify(self.get_type_of_node(lhs), self.get_type_of_node(rhs)) {
            Ok(unified) => {
                let unified = reg.substitute(lhs_tvar, unified);
                let unified = reg.substitute(rhs_tvar, unified);
                Ok(unified)
            }
            Err(err) => Err(TypeError::OnNodes(
                Box::new(err),
                format!("{:?}", lhs),
                self.get_type_of_node(lhs),
                format!("{:?}", rhs),
                self.get_type_of_node(rhs),
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

    pub(crate) fn node_types(&self) -> HashMap<NodeKey<'ast>, Arc<Type>> {
        self.reg.borrow().node_types().clone()
    }
}

/// # About this [`Visitor`] implementation.
///
/// On [`Visitor::enter`] invokes [`InferType::infer_enter`].
/// On [`Visitor::exit`] invokes [`InferType::infer_exit`].
impl<'ast> Visitor<'ast> for TypeInferencer<'ast> {
    type Error = TypeError;

    fn enter<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        if let Some(node) = node.downcast_ref::<Statement>() {
            into_control_flow(self.infer_enter(node))?
        }

        if let Some(node) = node.downcast_ref::<Query>() {
            into_control_flow(self.infer_enter(node))?
        }

        if let Some(node) = node.downcast_ref::<Insert>() {
            into_control_flow(self.infer_enter(node))?
        }

        if let Some(node) = node.downcast_ref::<Delete>() {
            into_control_flow(self.infer_enter(node))?
        }

        if let Some(node) = node.downcast_ref::<Expr>() {
            into_control_flow(self.infer_enter(node))?;
        }

        if let Some(node) = node.downcast_ref::<SetExpr>() {
            into_control_flow(self.infer_enter(node))?
        }

        if let Some(node) = node.downcast_ref::<Select>() {
            into_control_flow(self.infer_enter(node))?
        }

        if let Some(node) = node.downcast_ref::<Vec<SelectItem>>() {
            into_control_flow(self.infer_enter(node))?
        }

        if let Some(node) = node.downcast_ref::<SelectItem>() {
            into_control_flow(self.infer_enter(node))?
        }

        if let Some(node) = node.downcast_ref::<Function>() {
            into_control_flow(self.infer_enter(node))?
        }

        if let Some(node) = node.downcast_ref::<Values>() {
            into_control_flow(self.infer_enter(node))?
        }

        if let Some(node) = node.downcast_ref::<sqlparser::ast::Value>() {
            into_control_flow(self.infer_enter(node))?
        }

        ControlFlow::Continue(())
    }

    fn exit<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        if let Some(node) = node.downcast_ref::<Statement>() {
            into_control_flow(self.infer_exit(node))?
        }

        if let Some(node) = node.downcast_ref::<Query>() {
            into_control_flow(self.infer_exit(node))?
        }

        if let Some(node) = node.downcast_ref::<Insert>() {
            into_control_flow(self.infer_exit(node))?
        }

        if let Some(node) = node.downcast_ref::<Delete>() {
            into_control_flow(self.infer_exit(node))?
        }

        if let Some(node) = node.downcast_ref::<Expr>() {
            into_control_flow(self.infer_exit(node))?
        }

        if let Some(node) = node.downcast_ref::<SetExpr>() {
            into_control_flow(self.infer_exit(node))?
        }

        if let Some(node) = node.downcast_ref::<Select>() {
            into_control_flow(self.infer_exit(node))?
        }

        if let Some(node) = node.downcast_ref::<Vec<SelectItem>>() {
            into_control_flow(self.infer_exit(node))?
        }

        if let Some(node) = node.downcast_ref::<SelectItem>() {
            into_control_flow(self.infer_exit(node))?
        }

        if let Some(node) = node.downcast_ref::<Function>() {
            into_control_flow(self.infer_exit(node))?
        }

        if let Some(node) = node.downcast_ref::<Values>() {
            into_control_flow(self.infer_exit(node))?
        }

        if let Some(node) = node.downcast_ref::<sqlparser::ast::Value>() {
            into_control_flow(self.infer_exit(node))?
        }

        ControlFlow::Continue(())
    }
}

#[cfg(test)]
#[allow(dead_code)]
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
