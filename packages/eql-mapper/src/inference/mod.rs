mod function_signature;
mod infer_type;
mod infer_type_impls;
mod registry;
mod type_error;
mod type_variables;

pub mod unifier;

use unifier::*;

use std::{
    cell::RefCell,
    collections::{HashMap, HashSet},
    marker::PhantomData,
    ops::ControlFlow,
    rc::Rc,
    sync::Arc,
};

use itertools::Itertools;

use infer_type::InferType;
use sqlparser::ast::{
    Delete, Expr, Function, FunctionArguments, Insert, Query, Select, SetExpr, Statement, Value,
};
use sqltk::{into_control_flow, Break, Semantic, Visitable, Visitor};

use crate::{Schema, Scope};

pub(crate) use function_signature::*;
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
/// - [`Function`]
/// - [`FunctionArguments`]
#[derive(Debug)]
pub struct TypeInferencer<'ast> {
    /// A snapshot of the the database schema - used by `TypeInferencer`'s [`InferType`] impls.
    schema: Arc<Schema>,

    // The lexical scope - used for resolving identifiers & wildcard expansions in `TypeInferencer`'s [`InferType`] impls.
    scope: Rc<RefCell<Scope>>,

    /// Associates types with AST nodes.
    reg: Rc<RefCell<TypeRegistry<'ast>>>,

    /// Implements the type-unification algorithm.
    unifier: RefCell<unifier::Unifier>,

    /// The types of all placeholder nodes
    param_types: Vec<(String, Rc<RefCell<unifier::Type>>)>,

    /// References to all of the literal nodes.
    literal_nodes: HashSet<NodeKey<'ast>>,

    _ast: PhantomData<&'ast ()>,
}

impl<'ast> TypeInferencer<'ast> {
    /// Create a new `TypeInferencer`.
    pub fn new(
        schema: impl Into<Arc<Schema>>,
        scope: impl Into<Rc<RefCell<Scope>>>,
        reg: impl Into<Rc<RefCell<TypeRegistry<'ast>>>>,
    ) -> Self {
        Self {
            schema: schema.into(),
            scope: scope.into(),
            reg: reg.into(),
            unifier: RefCell::new(unifier::Unifier::new()),
            param_types: Vec::with_capacity(16),
            literal_nodes: HashSet::with_capacity(64),
            _ast: PhantomData,
        }
    }

    /// Shorthand for calling `self.reg.borrow_mut().get_type(node)` in [`InferType`] implementations for `TypeInferencer`.
    pub(crate) fn get_type<N: Semantic>(&self, node: &'ast N) -> Rc<RefCell<unifier::Type>> {
        self.reg.borrow_mut().get_type(node)
    }

    pub(crate) fn get_type_by_node_key(&self, key: &NodeKey<'ast>) -> Option<Rc<RefCell<unifier::Type>>> {
        self.reg.borrow_mut().get_type_by_node_key(key)
    }

    pub(crate) fn node_types(&self) -> Result<HashMap<NodeKey<'ast>, unifier::Type>, TypeError> {
        self.try_resolve_all_types().map(|types| {
            types
                .iter()
                .map(|(k, v)| (k.clone(), v.borrow().clone()))
                .collect()
        })
    }

    pub(crate) fn param_types(&self) -> Result<HashMap<String, unifier::Scalar>, TypeError> {
        // For every param node, unify its type with the other param nodes that refer to the same param.
        let unified_params = self
            .param_types
            .iter()
            .into_grouping_map_by(|&(param, _)| param.clone())
            .fold_with(
                |_, _| Ok(unifier::Type::fresh_tvar()),
                |acc_ty, _param, (_, param_ty)| {
                    acc_ty.and_then(|acc_ty| self.unify(acc_ty, param_ty.clone()))
                },
            );

        // Filter down to the params that successfully unified
        let verified_unified_params: HashMap<String, unifier::Type> = unified_params
            .iter()
            .filter_map(|(param, ty)| {
                ty.as_ref()
                    .map(|ty| ((*param).clone(), ty.borrow().clone()))
                    .ok()
            })
            .collect();

        // Check we still have the correct number of params
        if unified_params.len() != verified_unified_params.len() {
            let mut failed_params: HashSet<String> = HashSet::new();
            failed_params.extend(unified_params.keys().cloned());
            failed_params.retain(|param| verified_unified_params.contains_key(param));

            return Err(TypeError::Params(failed_params));
        };

        // Check that every unified param type is a Scalar
        let scalars: HashMap<String, unifier::Scalar> = verified_unified_params
            .into_iter()
            .map(|(param, ty)| {
                if let unifier::Def::Constructor(unifier::Constructor::Scalar(scalar)) = &ty.0 {
                    Ok((param, (**scalar).clone()))
                } else {
                    Err(TypeError::NonScalarParam(param, ty.to_string()))
                }
            })
            .collect::<Result<_, _>>()?;

        Ok(scalars)
    }

    pub(crate) fn literal_nodes(&self) -> HashSet<NodeKey<'ast>> {
        self.literal_nodes.clone()
    }

    /// Shorthand for calling `self.unifier.borrow_mut().unify(left, right)` in [`InferType`] implementations for `TypeInferencer`.
    fn unify(
        &self,
        left: Rc<RefCell<Type>>,
        right: Rc<RefCell<Type>>,
    ) -> Result<Rc<RefCell<Type>>, TypeError> {
        self.unifier.borrow_mut().unify(left, right)
    }

    /// Shorthand for calling `self.reg.borrow().try_resolve_all_types()` in [`InferType`] implementations for `TypeInferencer`.
    pub(crate) fn try_resolve_all_types(
        &self,
    ) -> Result<HashMap<NodeKey<'ast>, Rc<RefCell<Type>>>, TypeError> {
        self.reg.borrow().try_resolve_all_types()
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

        if let Some(node) = node.downcast_ref::<Function>() {
            into_control_flow(self.infer_enter(node))?
        }

        if let Some(node) = node.downcast_ref::<FunctionArguments>() {
            into_control_flow(self.infer_enter(node))?
        }

        ControlFlow::Continue(())
    }

    fn exit<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        if let Some(node) = node.downcast_ref::<Statement>() {
            into_control_flow(self.infer_exit(node))?;
            {
                let ty = self.get_type(node);
                let ty_mut = &mut *ty.borrow_mut();
                into_control_flow(ty_mut.try_resolve())?;
            }
            into_control_flow(self.try_resolve_all_types())?
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

        if let Some(node) = node.downcast_ref::<Function>() {
            into_control_flow(self.infer_exit(node))?
        }

        if let Some(node) = node.downcast_ref::<FunctionArguments>() {
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
