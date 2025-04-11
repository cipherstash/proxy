use std::{collections::HashMap};

use sqlparser::ast::{Expr, Function, Ident, Select, SelectItem};
use sqltk::{Context, NodeKey, Visitable};

use crate::{EqlMapperError, SqlIdent, Type};

use super::{
    helpers, selector::{MatchTrailing, Selector}, Rule
};

pub struct UseEquivalentSqlFuncForEqlTypes<'a, 'ast> {
    node_types: &'a HashMap<NodeKey<'ast>, Type>,
}

impl<'a, 'ast> UseEquivalentSqlFuncForEqlTypes<'a, 'ast> {
    pub fn new(node_types: &'a HashMap<NodeKey<'ast>, Type>) -> Self {
        Self { node_types }
    }
}

impl<'a, 'ast> Rule<'ast> for UseEquivalentSqlFuncForEqlTypes<'a,'ast> {
    type Sel = MatchTrailing<(Select, Vec<SelectItem>, SelectItem, Expr)>;

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
            &mut |(select, _, _, _), expr| {
                // TODO: drop this IF (being in a GROUP BY should not matter)
                if !helpers::is_used_in_group_by_clause(&self.node_types, &select.group_by, original_node) {
                    if let Expr::Function(Function { name, .. }) = expr {
                        let f_name = name.0.last_mut().unwrap();

                        if SqlIdent(&*f_name) == SqlIdent(Ident::new("MIN")) {
                            *f_name = Ident::new("CS_MIN_V1");
                        }

                        if SqlIdent(&*f_name) == SqlIdent(Ident::new("MAX")) {
                            *f_name = Ident::new("CS_MAX_V1");
                        }
                    }
                }
                Ok(())
            },
        )
    }
}
