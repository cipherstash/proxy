mod infer_type;
mod infer_type_impls;
mod registry;
mod type_error;
mod type_variables;

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

use crate::{ParamTracker, ScopeTracker, TableResolver, ValueTracker};

pub(crate) use registry::*;
pub(crate) use type_error::*;
pub(crate) use type_variables::*;

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
#[derive(Debug)]
pub struct TypeInferencer<'ast> {
    /// A snapshot of the the database schema - used by `TypeInferencer`'s [`InferType`] impls.
    table_resolver: Arc<TableResolver>,

    // The lexical scope - for resolving projection columns & expanding wildcards.
    scope_tracker: Rc<RefCell<ScopeTracker<'ast>>>,

    /// Associates types with AST nodes.
    reg: Rc<RefCell<TypeRegistry<'ast>>>,

    /// Implements the type unification algorithm.
    unifier: Rc<RefCell<Unifier<'ast>>>,

    value_tracker: Rc<RefCell<ValueTracker<'ast>>>,

    param_tracker: Rc<RefCell<ParamTracker<'ast>>>,

    _ast: PhantomData<&'ast ()>,
}

impl<'ast> TypeInferencer<'ast> {
    /// Create a new `TypeInferencer`.
    pub fn new(
        table_resolver: impl Into<Arc<TableResolver>>,
        scope: impl Into<Rc<RefCell<ScopeTracker<'ast>>>>,
        reg: impl Into<Rc<RefCell<TypeRegistry<'ast>>>>,
        unifier: impl Into<Rc<RefCell<Unifier<'ast>>>>,
        value_tracker: impl Into<Rc<RefCell<ValueTracker<'ast>>>>,
        param_tracker: impl Into<Rc<RefCell<ParamTracker<'ast>>>>,
    ) -> Self {
        Self {
            table_resolver: table_resolver.into(),
            scope_tracker: scope.into(),
            reg: reg.into(),
            unifier: unifier.into(),
            value_tracker: value_tracker.into(),
            param_tracker: param_tracker.into(),
            _ast: PhantomData,
        }
    }

    /// Shorthand for calling `self.reg.borrow_mut().get_type(node)` in [`InferType`] implementations for `TypeInferencer`.
    pub(crate) fn get_type<N: AsNodeKey>(&self, node: &'ast N) -> TypeCell {
        self.reg.borrow_mut().get_or_init_type(node).clone()
    }

    pub(crate) fn get_type_by_node_key(&self, key: &NodeKey<'ast>) -> Option<TypeCell> {
        self.reg.borrow_mut().get_type_by_node_key(key).cloned()
    }

    pub(crate) fn param_types(&self) -> Result<HashMap<String, unifier::Value>, TypeError> {
        let param_tracker = self.param_tracker.borrow();
        let param_types = param_tracker.param_types();

        // Check that every unified param type is a Scalar
        let scalars: HashMap<String, unifier::Value> = param_types
            .into_iter()
            .map(|(param, ty)| {
                if let unifier::Type::Constructor(unifier::Constructor::Value(value)) =
                    &*ty.as_type()
                {
                    Ok((param.to_string(), value.clone()))
                } else {
                    Err(TypeError::NonScalarParam(
                        param.to_string(),
                        ty.as_type().to_string(),
                    ))
                }
            })
            .collect::<Result<_, _>>()?;

        Ok(scalars)
    }

    /// Tries to unify two types but does not record the result.
    /// Recording the result is up to the caller.
    #[must_use = "the result of unify must ultimately be associated with an AST node"]
    fn unify(&self, left: TypeCell, right: TypeCell) -> Result<TypeCell, TypeError> {
        self.unifier.borrow_mut().unify(left, right)
    }

    /// Unifies the types of two nodes with a third type and records the unification result.
    /// After a successful unification both nodes will refer to the same type.
    fn unify_nodes_with_type<N1: AsNodeKey, N2: AsNodeKey>(
        &self,
        left: &'ast N1,
        right: &'ast N2,
        ty: TypeCell,
    ) -> Result<TypeCell, TypeError> {
        let unifier = &mut *self.unifier.borrow_mut();
        let ty0 = unifier.unify(self.get_type(left), self.get_type(right))?;
        let ty1 = unifier.unify(ty0, ty)?;
        Ok(ty1)
    }

    /// Unifies the type of a node with a second type and records the unification result.
    fn unify_node_with_type<N: AsNodeKey>(
        &self,
        node: &'ast N,
        ty: TypeCell,
    ) -> Result<TypeCell, TypeError> {
        let unifier = &mut *self.unifier.borrow_mut();
        unifier.unify(self.get_type(node), ty)
    }

    /// Unifies the types of two nodes with each other.
    /// After a successful unification both nodes will refer to the same type.
    fn unify_nodes<N1: AsNodeKey + Debug, N2: AsNodeKey + Debug>(
        &self,
        left: &'ast N1,
        right: &'ast N2,
    ) -> Result<TypeCell, TypeError> {
        match self
            .unifier
            .borrow_mut()
            .unify(self.get_type(left), self.get_type(right))
        {
            Ok(ty) => Ok(ty),
            Err(err) => Err(TypeError::OnNodes(
                Box::new(err),
                format!("{:?}", left),
                self.get_type(left),
                format!("{:?}", right),
                self.get_type(right),
            )),
        }
    }

    fn unify_all_with_type<N: Debug + AsNodeKey>(
        &self,
        nodes: &'ast [N],
        ty: TypeCell,
    ) -> Result<TypeCell, TypeError> {
        nodes
            .iter()
            .try_fold(ty, |ty, node| self.unify_node_with_type(node, ty))
    }

    pub(crate) fn fresh_tvar(&self) -> TypeCell {
        self.reg.borrow_mut().fresh_tvar()
    }

    pub(crate) fn node_types(&self) -> HashMap<NodeKey<'ast>, TypeCell> {
        self.reg.borrow_mut().take_node_types()
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
