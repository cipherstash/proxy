use super::helpers::cast_as_encrypted;
use super::TransformationRule;
use crate::{EqlMapperError, Type};
use sqltk::parser::ast::{Expr, Value, ValueWithSpan};
use sqltk::parser::tokenizer::Span;
use sqltk::{NodeKey, NodePath, Visitable};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug)]
pub struct CastParamsAsEncrypted<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> CastParamsAsEncrypted<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }
}

impl<'ast> TransformationRule<'ast> for CastParamsAsEncrypted<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if self.would_edit(node_path, target_node) {
            if let Some(
                expr @ Expr::Value(ValueWithSpan {
                    value: Value::Placeholder(_),
                    ..
                }),
            ) = target_node.downcast_mut()
            {
                let to_wrap = std::mem::replace(
                    expr,
                    Expr::Value(ValueWithSpan {
                        value: Value::Null,
                        span: Span::empty(),
                    }),
                );
                let Expr::Value(ValueWithSpan {
                    value: value @ Value::Placeholder(_),
                    ..
                }) = to_wrap
                else {
                    unreachable!("the Expr is known to be Expr::Value(ValueWithSpan::{{ value: Value::Placeholder(_), .. }})")
                };

                *expr = cast_as_encrypted(value);
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((
            node @ Expr::Value(ValueWithSpan {
                value: Value::Placeholder(_),
                ..
            }),
        )) = node_path.last_1_as()
        {
            if let Some(Type::Value(crate::Value::Eql(_))) =
                self.node_types.get(&NodeKey::new(node))
            {
                return true;
            }
        }
        false
    }
}
