use std::{collections::HashMap, sync::Arc};

use sqlparser::ast::{Expr, Function, Ident, Select, SelectItem};
use sqltk::{NodeKey, NodePath, Visitable};

use crate::{EqlMapperError, SqlIdent, Type};

use super::{helpers, TransformationRule};

#[derive(Debug)]
pub struct UseEquivalentSqlFuncForEqlTypes<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> UseEquivalentSqlFuncForEqlTypes<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }
}

impl<'ast> TransformationRule<'ast> for UseEquivalentSqlFuncForEqlTypes<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<(), EqlMapperError> {
        if let Some((select, _select_items, _select_item, expr)) =
            node_path.last_4_as::<Select, Vec<SelectItem>, SelectItem, Expr>()
        {
            // TODO: drop this IF (being in a GROUP BY should not matter)
            if !helpers::is_used_in_group_by_clause(&self.node_types, &select.group_by, expr) {
                let target_node = target_node.downcast_mut::<Expr>().unwrap();
                if let Expr::Function(Function { name, .. }) = target_node {
                    let f_name = name.0.last_mut().unwrap();

                    if SqlIdent(&*f_name) == SqlIdent(Ident::new("MIN")) {
                        *f_name = Ident::new("CS_MIN_V1");
                    }

                    if SqlIdent(&*f_name) == SqlIdent(Ident::new("MAX")) {
                        *f_name = Ident::new("CS_MAX_V1");
                    }
                }
            }
        }

        Ok(())
    }
}
