use std::{cell::RefCell, convert::Infallible, ops::ControlFlow, rc::Rc};

use sqlparser::ast::Expr;
use sqltk::{Break, NodeKey, Visitable, Visitor};

use crate::{
    unifier::{Type, TypeVar},
    TypeRegistry,
};

/// [`Visitor`] implementation that records a reference to every [`Value`] node that is encountered during AST traversal
/// and provides an API for interrogating the recorded nodes.
#[derive(Debug)]
pub struct ValueTracker<'ast> {
    registry: Rc<RefCell<TypeRegistry<'ast>>>,
    values: Vec<NodeKey<'ast>>,
}

impl<'ast> ValueTracker<'ast> {
    pub(crate) fn new(registry: impl Into<Rc<RefCell<TypeRegistry<'ast>>>>) -> Self {
        Self {
            registry: registry.into(),
            values: Vec::with_capacity(64),
        }
    }

    /// Checks if any of the recorded [`Value`] is of type `Type::Var(tvar)`
    pub(crate) fn exists_value_with_type_var(&self, tvar: TypeVar) -> bool {
        let reg = self.registry.borrow();
        for key in &self.values {
            if let Some(ty) = reg.get_type_by_node_key(key) {
                if let Type::Var(found_var) = &*ty.as_type() {
                    if *found_var == tvar {
                        return true;
                    }
                }
            }
        }
        false
    }
}

impl<'ast> Visitor<'ast> for ValueTracker<'ast> {
    type Error = Infallible;

    fn enter<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        if let Some(Expr::Value(_)) = node.downcast_ref::<Expr>() {
            self.values.push(node.as_node_key());
            return ControlFlow::Break(Break::SkipChildren);
        }

        ControlFlow::Continue(())
    }
}
