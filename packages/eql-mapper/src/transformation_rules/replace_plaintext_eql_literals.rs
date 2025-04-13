use std::{collections::HashMap, rc::Rc};

use sqlparser::ast::Expr;
use sqltk::{Context, NodeKey, Visitable};

use crate::{EqlMapperError, Type};

use super::{
    selector::{MatchTarget, Selector},
    Rule,
};

#[derive(Debug)]
pub struct ReplacePlaintextEqlLiterals<'ast> {
    node_types: Rc<HashMap<NodeKey<'ast>, Type>>,
    encrypted_literals: HashMap<NodeKey<'ast>, Expr>,
}

impl<'ast> ReplacePlaintextEqlLiterals<'ast> {
    pub fn new(
        node_types: Rc<HashMap<NodeKey<'ast>, Type>>,
        encrypted_literals: HashMap<NodeKey<'ast>, Expr>,
    ) -> Self {
        Self {
            node_types,
            encrypted_literals,
        }
    }
}

impl<'ast> Rule<'ast> for ReplacePlaintextEqlLiterals<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        ctx: &Context<'ast>,
        source_node: &'ast N,
        target_node: N,
    ) -> Result<N, EqlMapperError> {
        MatchTarget::<Expr>::on_match_then(
            ctx,
            source_node,
            target_node,
            &mut |source_expr, mut target_expr| {
                if let Some(replacement) =
                    self.encrypted_literals.remove(&NodeKey::new(source_expr))
                {
                    *&mut target_expr = replacement;
                }
                Ok(target_expr)
            },
        )
    }
}
