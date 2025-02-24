use crate::inference::TypeError;
use crate::{NodeKey, TypeRegistry};
use sqlparser::ast::OrderByExpr;
use sqltk::{Break, Visitable, Visitor};
use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt::Debug;
use std::ops::ControlFlow;
use std::rc::Rc;

#[derive(Debug)]
pub struct EqlFunctionTracker<'ast> {
    reg: Rc<RefCell<TypeRegistry<'ast>>>,
    pub to_wrap: HashSet<NodeKey<'ast>>,
}

impl<'ast> EqlFunctionTracker<'ast> {
    pub fn new(reg: impl Into<Rc<RefCell<TypeRegistry<'ast>>>>) -> Self {
        Self {
            reg: reg.into(),
            to_wrap: HashSet::new(),
        }
    }
}

#[derive(thiserror::Error, PartialEq, Eq, Debug)]
pub enum EqlFunctionTrackerError {
    #[error(transparent)]
    _TypeError(Box<TypeError>),
}

impl<'ast> Visitor<'ast> for EqlFunctionTracker<'ast> {
    type Error = EqlFunctionTrackerError;

    fn enter<N: Visitable>(&mut self, _node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        ControlFlow::Continue(())
    }

    fn exit<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        if let Some(node) = node.downcast_ref::<OrderByExpr>() {
            let node_key = NodeKey::new(&node.expr);

            if let Some(type_cell) = self.reg.borrow().get_type_by_node_key(&node_key) {
                if type_cell.is_eql_value() {
                    self.to_wrap.insert(node_key);
                }
            }
        }

        ControlFlow::Continue(())
    }
}
