use std::{collections::HashMap, rc::Rc};

use sqlparser::ast::{Expr, GroupByExpr, Ident, ObjectName};
use sqltk::{Context, NodeKey, Visitable};

use crate::{EqlMapperError, Type, Value};

use super::{
    helpers,
    selector::{MatchTrailing, Selector},
    Rule,
};

pub struct GroupByEqlCol<'ast> {
    node_types: Rc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> GroupByEqlCol<'ast> {
    pub fn new(node_types: Rc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }
}

impl<'ast> Rule<'ast> for GroupByEqlCol<'ast> {
    type Sel = MatchTrailing<(GroupByExpr, Vec<Expr>, Expr)>;

    fn apply<'ast_new: 'ast, N0: Visitable>(
        &mut self,
        ctx: &Context<'ast>,
        original_node: &'ast N0,
        target_node: &'ast_new mut N0,
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
