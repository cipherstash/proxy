use std::{collections::HashMap, sync::Arc};

use sqltk::{NodeKey, NodePath, Visitable};
use sqltk_parser::ast::{Expr, Function, Ident, Select, SelectItem};

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
    ) -> Result<bool, EqlMapperError> {
        if self.would_edit(node_path, target_node) {
            if let Some((_select, _select_items, _select_item, _expr)) =
                node_path.last_4_as::<Select, Vec<SelectItem>, SelectItem, Expr>()
            {
                let target_node = target_node.downcast_mut::<Expr>().unwrap();
                if let Expr::Function(Function { name, .. }) = target_node {
                    let f_name = name.0.last_mut().unwrap();

                    if SqlIdent(&*f_name) == SqlIdent(Ident::new("MIN")) {
                        *f_name = Ident::new("CS_MIN_V1");
                    }

                    if SqlIdent(&*f_name) == SqlIdent(Ident::new("MAX")) {
                        *f_name = Ident::new("CS_MAX_V1");
                    }

                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, target_node: &N) -> bool {
        if let Some((select, _select_items, _select_item, expr)) =
            node_path.last_4_as::<Select, Vec<SelectItem>, SelectItem, Expr>()
        {
            if !helpers::is_used_in_group_by_clause(&self.node_types, &select.group_by, expr) {
                let target_node = target_node.downcast_ref::<Expr>().unwrap();
                if let Expr::Function(Function { name, .. }) = target_node {
                    let f_name = name.0.last().unwrap();

                    if SqlIdent(f_name) == SqlIdent(Ident::new("MIN")) {
                        return true;
                    }

                    if SqlIdent(f_name) == SqlIdent(Ident::new("MAX")) {
                        return true;
                    }
                }
            }
        }

        false
    }
}
