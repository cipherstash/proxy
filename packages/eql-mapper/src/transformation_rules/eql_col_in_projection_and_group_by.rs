use std::{collections::HashMap, rc::Rc};

use sqlparser::ast::{Expr, Ident, ObjectName, Select, SelectItem};
use sqltk::{NodeKey, NodePath, Visitable};

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
pub struct EqlColInProjectionAndGroupBy<'ast> {
    node_types: Rc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> EqlColInProjectionAndGroupBy<'ast> {
    pub(crate) fn new(node_types: Rc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }
}

impl<'ast> TransformationRule<'ast> for EqlColInProjectionAndGroupBy<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<(), EqlMapperError> {
        if let Some((select, _select_items, _select_item, expr)) =
            node_path.last_4_as::<Select, Vec<SelectItem>, SelectItem, Expr>()
        {
            if is_used_in_group_by_clause(&*self.node_types, &select.group_by, expr) {
                let target_node: &mut Expr = target_node.downcast_mut().unwrap();
                *target_node = wrap_in_1_arg_function(
                    expr.clone(),
                    ObjectName(vec![Ident::new("CS_GROUPED_VALUE_V1")]),
                );
            }
        }

        Ok(())
    }
}
