use std::{cell::RefCell, collections::HashMap, ops::ControlFlow, rc::Rc};

use sqlparser::ast::{Expr, Value};
use sqltk::{Break, Visitable, Visitor};

use crate::{
    unifier::{TypeCell, Unifier}, TypeError, TypeRegistry
};

/// [`Visitor`] implementation that records a reference to every [`Value`] node that is encountered during AST traversal
/// and provides an API for interrogating the recorded nodes.
#[derive(Debug)]
pub struct ParamTracker<'ast> {
    registry: Rc<RefCell<TypeRegistry<'ast>>>,
    unifier: Rc<RefCell<Unifier<'ast>>>,

    /// The `(param name, type)` of all of the `Expr::Value(Value::Placeholder)` nodes that were discovered in the AST.
    param_nodes: HashMap<&'ast String, TypeCell>,
}

impl<'ast> ParamTracker<'ast> {
    pub(crate) fn new(
        registry: impl Into<Rc<RefCell<TypeRegistry<'ast>>>>,
        unifier: impl Into<Rc<RefCell<Unifier<'ast>>>>,
    ) -> Self {
        Self {
            registry: registry.into(),
            unifier: unifier.into(),
            param_nodes: HashMap::with_capacity(64),
        }
    }

    pub(crate) fn param_types(&self) -> &HashMap<&'ast String, TypeCell> {
        &self.param_nodes
    }
}

impl<'ast> Visitor<'ast> for ParamTracker<'ast> {
    type Error = TypeError;

    fn enter<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        if let Some(Expr::Value(Value::Placeholder(param_name))) = node.downcast_ref::<Expr>() {
            let mut reg = self.registry.borrow_mut();

            let left = reg.get_or_init_type(node).clone();
            drop(reg);
            let right = self.param_nodes.entry(param_name).or_insert(left.clone()).clone();

            // Unify its type with any other placeholder that refers to the same param name
            match self.unifier.borrow_mut().unify(left, right) {
                Ok(_) => return ControlFlow::Continue(()),
                Err(err) => return ControlFlow::Break(Break::Err(err))
            }
        }

        ControlFlow::Continue(())
    }
}
