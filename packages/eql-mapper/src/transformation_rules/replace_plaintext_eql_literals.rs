use std::{cell::RefCell, collections::HashMap, rc::Rc};

use sqlparser::ast::Expr;
use sqltk::{Context, NodeKey, Visitable};

use crate::{EqlMapperError, Type};

use super::{
    selector::{MatchTarget, Selector},
    Rule,
};

pub struct ReplacePlaintextEqlLiterals<'ast> {
    node_types: Rc<HashMap<NodeKey<'ast>, Type>>,
    encrypted_literals: Rc<RefCell<HashMap<NodeKey<'ast>, Expr>>>,
}

impl<'ast> ReplacePlaintextEqlLiterals<'ast> {
    pub fn new(
        node_types: Rc<HashMap<NodeKey<'ast>, Type>>,
        encrypted_literals: Rc<RefCell<HashMap<NodeKey<'ast>, Expr>>>,
    ) -> Self {
        Self {
            node_types,
            encrypted_literals,
        }
    }
}

impl<'ast> Rule<'ast> for ReplacePlaintextEqlLiterals<'ast> {
    type Sel = MatchTarget<Expr>;

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
            &mut |source_expr, target_expr| {
                if let Some(replacement) = self
                    .encrypted_literals
                    .borrow_mut()
                    .remove(&NodeKey::new(source_expr))
                {
                    *target_expr = replacement;
                }
                Ok(())
            },
        )
    }
}
