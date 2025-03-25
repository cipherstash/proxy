mod infer_type;
mod infer_type_impls;
mod node_key;
mod registry;
mod semantic_eq;
mod type_error;
mod type_variables;

pub mod unifier;

use unifier::*;

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    fmt::Debug,
    marker::PhantomData,
    ops::ControlFlow,
    rc::Rc,
    sync::Arc,
};

use itertools::Itertools;

use infer_type::InferType;
use sqlparser::ast::{
    Delete, Expr, Function, Insert, Query, Select, SelectItem, SetExpr, Statement, Value, Values,
};
use sqltk::{into_control_flow, Break, Semantic, Visitable, Visitor};

use crate::{ScopeTracker, TableResolver};

pub use node_key::*;
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
    unifier: Rc<RefCell<unifier::Unifier<'ast>>>,

    /// The parameter identifiers & types of `Expr::Value(Value::Placeholder(_))` nodes found in a [`Statement`].
    param_types: Vec<(String, TypeCell)>,

    /// References to all of the `Expr::Value(_)` nodes found in a [`Statement`] - except for
    /// `Expr::Value(Value::Placeholder(_))` nodes.
    literal_nodes: HashSet<NodeKey<'ast>>,

    _ast: PhantomData<&'ast ()>,
}

impl<'ast> TypeInferencer<'ast> {
    /// Create a new `TypeInferencer`.
    pub fn new(
        table_resolver: impl Into<Arc<TableResolver>>,
        scope: impl Into<Rc<RefCell<ScopeTracker<'ast>>>>,
        reg: impl Into<Rc<RefCell<TypeRegistry<'ast>>>>,
        unifier: impl Into<Rc<RefCell<unifier::Unifier<'ast>>>>,
    ) -> Self {
        Self {
            table_resolver: table_resolver.into(),
            scope_tracker: scope.into(),
            reg: reg.into(),
            unifier: unifier.into(),
            param_types: Vec::with_capacity(16),
            literal_nodes: HashSet::with_capacity(64),
            _ast: PhantomData,
        }
    }

    /// Shorthand for calling `self.reg.borrow_mut().get_type(node)` in [`InferType`] implementations for `TypeInferencer`.
    pub(crate) fn get_type<N: Semantic>(&self, node: &'ast N) -> TypeCell {
        self.reg.borrow_mut().get_type(node).clone()
    }

    pub(crate) fn get_type_by_node_key(&self, key: &NodeKey<'ast>) -> Option<TypeCell> {
        self.reg.borrow_mut().get_type_by_node_key(key).cloned()
    }

    pub(crate) fn param_types(&self) -> Result<HashMap<String, unifier::Value>, TypeError> {
        // For every param node, unify its type with the other param nodes that refer to the same param.
        let unified_params = self
            .param_types
            .iter()
            .into_grouping_map_by(|&(param, _)| param.clone())
            .fold_with(
                |_, _| Ok(self.fresh_tvar()),
                |acc_ty, _param, (_, param_ty)| {
                    acc_ty.and_then(|acc_ty| self.unify(acc_ty, param_ty.clone()))
                },
            );

        // Filter down to the params that successfully unified
        let verified_unified_params: HashMap<String, TypeCell> = unified_params
            .iter()
            .filter_map(|(param, ty)| ty.as_ref().map(|ty| ((*param).clone(), ty.clone())).ok())
            .collect();

        // Check we still have the correct number of params
        if unified_params.len() != verified_unified_params.len() {
            let mut failed_params: HashSet<String> = HashSet::new();
            failed_params.extend(unified_params.keys().cloned());
            failed_params.retain(|param| verified_unified_params.contains_key(param));

            return Err(TypeError::Params(failed_params));
        };

        // Check that every unified param type is a Scalar
        let scalars: HashMap<String, unifier::Value> = verified_unified_params
            .into_iter()
            .map(|(param, ty)| {
                if let unifier::Type::Constructor(unifier::Constructor::Value(value)) =
                    &*ty.as_type()
                {
                    Ok((param, value.clone()))
                } else {
                    Err(TypeError::NonScalarParam(param, ty.as_type().to_string()))
                }
            })
            .collect::<Result<_, _>>()?;

        Ok(scalars)
    }

    pub(crate) fn literal_nodes(&self) -> HashSet<NodeKey<'ast>> {
        self.literal_nodes.clone()
    }

    /// Tries to unify two types but does not record the result.
    /// Recording the result is up to the caller.
    #[must_use = "the result of unify must ultimately be associated with an AST node"]
    fn unify(&self, left: TypeCell, right: TypeCell) -> Result<TypeCell, TypeError> {
        self.unifier.borrow_mut().unify(left, right)
    }

    /// Unifies the types of two nodes with a third type and records the unification result.
    /// After a successful unification both nodes will refer to the same type.
    fn unify_nodes_with_type<N1: Semantic, N2: Semantic>(
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
    fn unify_node_with_type<N: Semantic>(
        &self,
        node: &'ast N,
        ty: TypeCell,
    ) -> Result<TypeCell, TypeError> {
        let unifier = &mut *self.unifier.borrow_mut();
        unifier.unify(self.get_type(node), ty)
    }

    /// Unifies the types of two nodes with each other.
    /// After a successful unification both nodes will refer to the same type.
    fn unify_nodes<N1: Semantic + Debug, N2: Semantic + Debug>(
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

    fn unify_all_with_type<N: Semantic + Debug>(
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
            if let Expr::Value(value) = node {
                match value {
                    Value::Placeholder(param) => {
                        self.param_types.push((param.clone(), self.get_type(node)));
                    }
                    _ => {
                        self.literal_nodes.insert(NodeKey::new(node));
                    }
                }
            }
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
