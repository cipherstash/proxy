use std::{collections::HashMap};

use sqlparser::ast::{Expr, GroupByExpr, Ident, ObjectName};
use sqltk::{Context, NodeKey, Visitable};

use crate::{EqlMapperError, Type, Value};

use super::{
    helpers,
    selector::{MatchTrailing, Selector},
    Rule,
};

pub struct GroupByEqlCol<'a, 'ast> {
    node_types: &'a HashMap<NodeKey<'ast>, Type>,
}

impl<'a, 'ast> GroupByEqlCol<'a, 'ast> {
    pub fn new(node_types: &'a HashMap<NodeKey<'ast>, Type>) -> Self {
        Self { node_types }
    }
}

impl<'a, 'ast> Rule<'ast> for GroupByEqlCol<'a, 'ast> {
    type Sel = MatchTrailing<(GroupByExpr, Vec<Expr>, Expr)>;

    fn apply<N0: Visitable>(
        &mut self,
        ctx: &Context<'ast>,
        original_node: &'ast N0,
        target_node: &mut N0,
    ) -> Result<(), EqlMapperError> {
        Self::Sel::on_match_then(
            ctx,
            original_node,
            target_node,
            &mut |(_group_by, _exprs, _expr), expr| {
                if let Some(Type::Value(Value::Eql(_))) =
                    self.node_types.get(&original_node.as_node_key())
                {
                    *expr = helpers::wrap_in_1_arg_function(
                        expr.clone(),
                        ObjectName(vec![Ident::new("CS_ORE_64_8_V1")]),
                    );
                }

                Ok(())
            },
        )
    }
}
