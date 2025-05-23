use std::{collections::HashMap, mem, sync::Arc};

use sqltk::parser::ast::{
    helpers::attached_token::AttachedToken, Expr, Ident, ObjectName, OrderByExpr,
};
use sqltk::parser::tokenizer::{Span, Token, TokenWithSpan};
use sqltk::{NodeKey, NodePath, Visitable};

use crate::{EqlMapperError, Type, Value};

use super::{helpers::wrap_in_1_arg_function, TransformationRule};

/// When an [`Expr`] of a [`SelectItem`] has an EQL type and that EQL type is used in a `GROUP BY` clause then
/// this rule wraps the `Expr` in a call to `eql_v2.grouped_value`.
///
/// # Example
///
/// ```sql
/// -- before mapping
/// SELECT eql_col FROM some_table GROUP BY eql_col;
///
/// -- after mapping
/// SELECT eql_v2.grouped_value(eql_col) AS eql_col FROM some_table GROUP BY eql_v2.cs_ore_64_8(eql_col);
/// --     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^     ^^^^^^^                          ^^^^^^^^^^^^^^^^^^^^^^^^^^^
/// --                 ^                       ^                                       ^
/// --                 |                       |                                       |
/// --     Changed by this rule       Preserve effective aliases          Changed by rule GroupByEqlCol
/// ```
#[derive(Debug)]
pub struct WrapEqlColsInOrderByWithOreFn<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> WrapEqlColsInOrderByWithOreFn<'ast> {
    pub(crate) fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }
}

impl<'ast> TransformationRule<'ast> for WrapEqlColsInOrderByWithOreFn<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if self.would_edit(node_path, target_node) {
            if let Some((_order_by_expr,)) = node_path.last_1_as::<OrderByExpr>() {
                let target_node = target_node.downcast_mut::<OrderByExpr>().unwrap();

                let expr_to_wrap = mem::replace(
                    &mut target_node.expr,
                    Expr::Wildcard(AttachedToken(TokenWithSpan::new(Token::EOF, Span::empty()))),
                );

                target_node.expr = wrap_in_1_arg_function(
                    expr_to_wrap,
                    ObjectName(vec![
                        Ident::new("eql_v2"),
                        Ident::new("ore_block_u64_8_256"),
                    ]),
                );

                return Ok(true);
            }
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((order_by_expr,)) = node_path.last_1_as::<OrderByExpr>() {
            let node_key = NodeKey::new(&order_by_expr.expr);

            if let Some(ty) = self.node_types.get(&node_key) {
                return matches!(ty, Type::Value(Value::Eql(_)));
            }
        }

        false
    }
}
