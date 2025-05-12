use std::collections::HashMap;
use std::sync::Arc;

use sqltk::parser::ast::{Expr, Value};
use sqltk::{NodeKey, NodePath, Visitable};

use crate::{EqlMapperError, Type};

use super::helpers::make_row_expression;
use super::TransformationRule;

#[derive(Debug)]
pub struct WrapEqlParamsInRow<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> WrapEqlParamsInRow<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }
}

impl<'ast> TransformationRule<'ast> for WrapEqlParamsInRow<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if self.would_edit(node_path, target_node) {
            if let Some(expr @ Expr::Value(Value::Placeholder(_))) = target_node.downcast_mut() {
                let to_wrap = std::mem::replace(expr, Expr::Value(Value::Null));
                let Expr::Value(value @ Value::Placeholder(_)) = to_wrap else {
                    unreachable!("the Expr is known to be Expr::Value(Value::Placeholder(_))")
                };

                *expr = make_row_expression(value);
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((node @ Expr::Value(Value::Placeholder(_)),)) = node_path.last_1_as() {
            if let Some(Type::Value(crate::Value::Eql(_))) =
                self.node_types.get(&NodeKey::new(node))
            {
                return true;
            }
        }
        false
    }
}
