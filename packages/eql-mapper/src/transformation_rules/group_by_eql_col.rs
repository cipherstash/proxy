use std::{collections::HashMap, mem, sync::Arc};

use sqlparser::ast::{Expr, GroupByExpr, Ident, ObjectName};
use sqltk::{NodeKey, NodePath, Visitable};

use crate::{EqlMapperError, Type, Value};

use super::{
    helpers,
    TransformationRule,
};

#[derive(Debug)]
pub struct GroupByEqlCol<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> GroupByEqlCol<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }
}

impl<'ast> TransformationRule<'ast> for GroupByEqlCol<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<(), EqlMapperError> {
        if let Some((_group_by_expr, _exprs, expr)) =
            node_path.last_3_as::<GroupByExpr, Vec<Expr>, Expr>()
        {
            if let Some(Type::Value(Value::Eql(_))) =
                self.node_types.get(&NodeKey::new(expr))
            {
                let target_node = target_node.downcast_mut::<Expr>().unwrap();

                // Nodes are modified starting from the leaf nodes, to target_node *is* what we want to be wrapping.
                // So we steal the existing value and replace the original with a cheap placeholder (Expr::Wildcard).
                // Stealing the original subtree means we can avoid cloning it.
                let transformed_expr = mem::replace(target_node, Expr::Wildcard);

                *target_node = helpers::wrap_in_1_arg_function(
                    transformed_expr,
                    ObjectName(vec![Ident::new("CS_ORE_64_8_V1")]),
                );
            }
        }

        Ok(())
    }
}
