use std::{collections::HashMap, mem, sync::Arc};

use sqltk::parser::ast::{
    helpers::attached_token::AttachedToken, Expr, GroupByExpr, Ident, ObjectName,
};
use sqltk::parser::tokenizer::{Span, Token, TokenWithSpan};
use sqltk::{NodeKey, NodePath, Visitable};

use crate::{EqlMapperError, Type, Value};

use super::{helpers, TransformationRule};

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
    ) -> Result<bool, EqlMapperError> {
        if self.would_edit(node_path, &*target_node) {
            let target_node = target_node.downcast_mut::<Expr>().unwrap();

            // Nodes are modified starting from the leaf nodes, to target_node *is* what we want to be wrapping.
            // So we steal the existing value and replace the original with a cheap placeholder (Expr::Wildcard).
            // Stealing the original subtree means we can avoid cloning it.
            let transformed_expr = mem::replace(
                target_node,
                Expr::Wildcard(AttachedToken(TokenWithSpan::new(Token::EOF, Span::empty()))),
            );

            *target_node = helpers::wrap_in_1_arg_function(
                transformed_expr,
                ObjectName(vec![Ident::new("eql_v1"), Ident::new("ore_64_8_v1")]),
            );

            return Ok(true);
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((_group_by_expr, _exprs, expr)) =
            node_path.last_3_as::<GroupByExpr, Vec<Expr>, Expr>()
        {
            if let Some(Type::Value(Value::Eql(_))) = self.node_types.get(&NodeKey::new(expr)) {
                return true;
            }
        }

        false
    }
}
