use std::{collections::HashMap, mem, sync::Arc};

use sqlparser::ast::{Expr, Ident, ObjectName, OrderByExpr};
use sqltk::{NodeKey, NodePath, Visitable};

use crate::{EqlMapperError, Type, Value};

use super::{helpers::wrap_in_1_arg_function, TransformationRule};

/// When an [`Expr`] of a [`SelectItem`] has an EQL type and that EQL type is used in a `GROUP BY` clause then
/// this rule wraps the `Expr` in a call to `CS_GROUPED_VALUE_V1`.
///
/// # Example
///
/// ```sql
/// -- before mapping
/// SELECT eql_col FROM some_table GROUP BY eql_col;
///
/// -- after mapping
/// SELECT CS_GROUPED_VALUE_V1(eql_col) FROM some_table GROUP BY CS_ORE_64_8_V1(eql_col);
/// --     ^^^^^^^^^^^^^^^^^^^^^^^^^^^^                          ^^^^^^^^^^^^^^^^^^^^^^^
/// --                 ^                                                    ^
/// --                 |                                                    |
/// --     Changed by this rule                                Changed by rule `GroupByEqlCol`
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
    ) -> Result<(), EqlMapperError> {
        if let Some((order_by_expr,)) = node_path.last_1_as::<OrderByExpr>() {
            let node_key = NodeKey::new(order_by_expr);

            if let Some(ty) = self.node_types.get(&node_key) {
                if matches!(ty, Type::Value(Value::Eql(_))) {
                    let target_node = target_node.downcast_mut::<OrderByExpr>().unwrap();

                    let expr_to_wrap = mem::replace(&mut target_node.expr, Expr::Wildcard);

                    target_node.expr = wrap_in_1_arg_function(
                        expr_to_wrap,
                        ObjectName(vec![Ident::new("CS_ORE_64_8_V1")]),
                    );
                }
            }
        }

        Ok(())
    }
}
