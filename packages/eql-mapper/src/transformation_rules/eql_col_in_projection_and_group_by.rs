use std::{collections::HashMap, rc::Rc};

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
#[derive(Debug)]
pub struct EqlColInProjectionAndGroupBy<'ast> {
    node_types: Rc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> EqlColInProjectionAndGroupBy<'ast> {
    pub(crate) fn new(node_types: Rc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }
}

impl<'ast> Rule<'ast> for EqlColInProjectionAndGroupBy<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        ctx: &Context<'ast>,
        source_node: &'ast N,
        target_node: N,
    ) -> Result<N, EqlMapperError> {
        MatchTrailing::<(Select, Vec<SelectItem>, SelectItem, Expr)>::on_match_then(
            ctx,
            source_node,
            target_node,
            &mut |(select, _, _, _), mut expr| {
                if is_used_in_group_by_clause(&*self.node_types, &select.group_by, source_node) {
                    *&mut expr = wrap_in_1_arg_function(
                        expr.clone(),
                        ObjectName(vec![Ident::new("CS_GROUPED_VALUE_V1")]),
                    );
                }

                Ok(expr)
            },
        )
    }
}
