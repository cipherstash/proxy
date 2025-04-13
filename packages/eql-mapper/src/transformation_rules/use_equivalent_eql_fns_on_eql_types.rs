use std::{collections::HashMap, rc::Rc};

use sqlparser::ast::{Expr, Function, Ident, Select, SelectItem};
use sqltk::{Context, NodeKey, Visitable};

use crate::{EqlMapperError, SqlIdent, Type};

use super::{
    helpers, selector::{MatchTrailing, Selector}, Rule
};

#[derive(Debug)]
pub struct UseEquivalentSqlFuncForEqlTypes<'ast> {
    node_types: Rc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> UseEquivalentSqlFuncForEqlTypes<'ast> {
    pub fn new(node_types: Rc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }
}

impl<'ast> Rule<'ast> for UseEquivalentSqlFuncForEqlTypes<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        ctx: &Context<'ast>,
        original_node: &'ast N,
        target_node: N,
    ) -> Result<N, EqlMapperError> {
        MatchTrailing::<(Select, Vec<SelectItem>, SelectItem, Expr)>::on_match_then(
            ctx,
            original_node,
            target_node,
            &mut |(select, _, _, _), mut expr| {
                // TODO: drop this IF (being in a GROUP BY should not matter)
                if !helpers::is_used_in_group_by_clause(&self.node_types, &select.group_by, original_node) {
                    if let Expr::Function(Function { name, .. }) = &mut expr {
                        let f_name = name.0.last_mut().unwrap();

                        if SqlIdent(&*f_name) == SqlIdent(Ident::new("MIN")) {
                            *f_name = Ident::new("CS_MIN_V1");
                        }

                        if SqlIdent(&*f_name) == SqlIdent(Ident::new("MAX")) {
                            *f_name = Ident::new("CS_MAX_V1");
                        }
                    }
                }
                Ok(expr)
            },
        )
    }
}
