use std::{collections::HashMap, mem, rc::Rc};

use sqlparser::ast::{Expr, Ident, ObjectName, OrderByExpr, Select, SelectItem};
use sqltk::{Context, NodeKey, Visitable};

use crate::{EqlMapperError, Type, Value};

use super::{
    helpers::{is_used_in_group_by_clause, wrap_in_1_arg_function},
    selector::{MatchTarget, MatchTrailing, Selector},
    Rule,
};

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
pub struct OrderByExprWithEqlType<'ast> {
    node_types: Rc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> OrderByExprWithEqlType<'ast> {
    pub(crate) fn new(node_types: Rc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }
}

impl<'ast> Rule<'ast> for OrderByExprWithEqlType<'ast> {
    type Sel = MatchTarget<OrderByExpr>;

    fn apply<'ast_new: 'ast, N0: Visitable>(
        &mut self,
        ctx: &Context<'ast>,
        source_node: &'ast N0,
        target_node: &'ast_new mut N0,
    ) -> Result<(), EqlMapperError> {
        Self::Sel::on_match_then(
            ctx,
            source_node,
            target_node,
            &mut |source_order_by_expr, target_order_by_expr| {
                let node_key = NodeKey::new(source_order_by_expr);

                if let Some(ty) = self.node_types.get(&node_key) {
                    if matches!(ty, Type::Value(Value::Eql(_))) {
                        *&mut target_order_by_expr.expr = wrap_in_1_arg_function(
                            mem::replace(&mut target_order_by_expr.expr, Expr::Wildcard),
                            ObjectName(vec![Ident::new("CS_ORE_64_8_V1")]),
                        );
                    }
                }

                Ok(())
            },
        )
    }
}
