use std::{collections::HashMap, sync::Arc};

use sqltk::{NodeKey, NodePath, Visitable};
use sqltk_parser::ast::{Expr, Ident, ObjectName, Select, SelectItem};

use crate::{EqlMapperError, Type};

use super::{
    helpers::{is_used_in_group_by_clause, wrap_in_1_arg_function},
    TransformationRule,
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
#[derive(Debug)]
pub struct WrapGroupedEqlColInAggregateFn<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> WrapGroupedEqlColInAggregateFn<'ast> {
    pub(crate) fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }
}

impl<'ast> TransformationRule<'ast> for WrapGroupedEqlColInAggregateFn<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if self.would_edit(node_path, target_node) {
            if let Some((_select, _select_items, _select_item, expr)) =
                node_path.last_4_as::<Select, Vec<SelectItem>, SelectItem, Expr>()
            {
                let target_node: &mut Expr = target_node.downcast_mut().unwrap();
                *target_node = wrap_in_1_arg_function(
                    expr.clone(),
                    ObjectName(vec![Ident::new("CS_GROUPED_VALUE_V1")]),
                );

                return Ok(true);
            }
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((select, _select_items, _select_item, expr)) =
            node_path.last_4_as::<Select, Vec<SelectItem>, SelectItem, Expr>()
        {
            return is_used_in_group_by_clause(&self.node_types, &select.group_by, expr);
        }

        false
    }
}
