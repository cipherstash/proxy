use std::collections::HashMap;

use sqlparser::ast::{Expr, Ident, ObjectName, Select, SelectItem};
use sqltk::{Context, NodeKey, Visitable};

use crate::{EqlMapperError, Type};

use super::{
    helpers::{is_used_in_group_by_clause, wrap_in_1_arg_function},
    selector::{MatchTrailing, Selector},
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
pub struct EqlColInProjectionAndGroupBy<'a, 'ast> {
    node_types: &'a HashMap<NodeKey<'ast>, Type>,
}

impl<'a, 'ast> EqlColInProjectionAndGroupBy<'a, 'ast> {
    pub(crate) fn new(node_types: &'a HashMap<NodeKey<'ast>, Type>) -> Self {
        Self { node_types }
    }
}

impl<'a, 'ast> Rule<'ast> for EqlColInProjectionAndGroupBy<'a, 'ast> {
    type Sel = MatchTrailing<(Select, Vec<SelectItem>, SelectItem, Expr)>;

    fn apply<N0: Visitable>(
        &mut self,
        ctx: &Context<'ast>,
        source_node: &'ast N0,
        target_node: &mut N0,
    ) -> Result<(), EqlMapperError> {
        Self::Sel::on_match_then(
            ctx,
            source_node,
            target_node,
            &mut |(select, _, _, _), expr| {
                if is_used_in_group_by_clause(self.node_types, &select.group_by, source_node) {
                    *expr = wrap_in_1_arg_function(
                        expr.clone(),
                        ObjectName(vec![Ident::new("CS_GROUPED_VALUE_V1")]),
                    );
                }

                Ok(())
            },
        )
    }
}
