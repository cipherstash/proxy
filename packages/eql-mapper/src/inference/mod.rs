mod function_signature;
mod infer_type;
mod infer_type_impls;
mod registry;
mod type_error;
mod type_variables;
mod types;
mod unifier;

use std::{cell::RefCell, collections::HashMap, ops::ControlFlow, rc::Rc, sync::Arc};

use infer_type::InferType;
use sqlparser::ast::{
    Delete, Expr, Function, FunctionArguments, Insert, Query, Select, SetExpr, Statement,
};
use sqltk::{into_control_flow, Break, Semantic, Visitable, Visitor};

use crate::{Schema, Scope};

pub(crate) use function_signature::*;
pub(crate) use registry::*;
pub(crate) use type_error::*;
pub(crate) use type_variables::*;
pub(crate) use types::*;
pub(crate) use unifier::*;

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
pub struct TypeInferencer {
    /// A snapshot of the the database schema - used by `TypeInferencer`'s [`InferType`] impls.
    schema: Arc<Schema>,

    // The lexical scope - used for resolving identifiers & wildcard expansions in `TypeInferencer`'s [`InferType`] impls.
    scope: Rc<RefCell<Scope>>,

    /// Associates types with AST nodes.
    reg: Rc<RefCell<TypeRegistry>>,

    /// Implements the type-unification algorithm.
    unifier: RefCell<Unifier>,
}

impl TypeInferencer {
    /// Create a new `TypeInferencer`.
    pub fn new(
        schema: Arc<Schema>,
        scope: Rc<RefCell<Scope>>,
        reg: Rc<RefCell<TypeRegistry>>,
        unifier: RefCell<Unifier>,
    ) -> Self {
        Self {
            schema,
            scope,
            reg,
            unifier,
        }
    }

    /// Shorthand for calling `self.reg.borrow_mut().get_type(node)` in [`InferType`] implementations for `TypeInferencer`.
    pub(crate) fn get_type<N: Semantic>(&self, node: &N) -> Rc<RefCell<Type>> {
        self.reg.borrow_mut().get_type(node)
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
    fn try_resolve_all_types(&self) -> Result<HashMap<NodeKey, Rc<RefCell<Type>>>, TypeError> {
        self.reg.borrow().try_resolve_all_types()
    }
}

/// # About this [`Visitor`] implementation.
///
/// On [`Visitor::enter`] invokes [`InferType::infer_enter`].
/// On [`Visitor::exit`] invokes [`InferType::infer_exit`].
impl<'ast> Visitor<'ast> for TypeInferencer {
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
            into_control_flow(self.infer_enter(node))?
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
pub(crate) mod test_util {
    use sqltk::Visitable;

    use super::TypeInferencer;

    impl TypeInferencer {
        /// Dump all nodes from the registry to STDERR. Useful for debugging failing tests.
        pub(crate) fn dump_registry<N: Visitable>(&self, root_node: &N) {
            self.reg.borrow().dump_all_nodes(root_node);
        }
    }
}
