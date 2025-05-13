use std::{collections::HashMap, mem, sync::Arc};

use sqltk::parser::ast::{self, CastKind, DataType, Select, SelectItem};
use sqltk::parser::ast::{
    helpers::attached_token::AttachedToken, Expr, GroupByExpr, Ident, ObjectName,
};
use sqltk::parser::tokenizer::{Span, Token, TokenWithSpan};
use sqltk::{AsNodeKey, NodeKey, NodePath, Visitable};

use crate::{EqlMapperError, Type, Value};

use super::{helpers, TransformationRule};

#[derive(Debug)]
pub struct CastEqlColsInOuterProjectionToJsonb<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> CastEqlColsInOuterProjectionToJsonb<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }
}

impl<'ast> TransformationRule<'ast> for CastEqlColsInOuterProjectionToJsonb<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if self.would_edit(node_path, &*target_node) {
            if let Some((original_select_items,)) = node_path.last_1_as::<Vec<SelectItem>>() {
                if let Some(select_items) = target_node.downcast_mut::<Vec<SelectItem>>() {
                    for (original_select_item, select_item) in original_select_items.iter().zip(select_items) {
                        if matches!(
                            self.node_types.get(&NodeKey::new(&*original_select_item)),
                            Some(Type::Value(Value::Eql(_)))
                        ) {
                            match select_item {
                                SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, alias: _ } => {
                                    *expr = cast_expr_to_jsonb(std::mem::replace(
                                        expr,
                                        Expr::Value(ast::Value::Null),
                                    ))
                                }
                                SelectItem::QualifiedWildcard(
                                    object_name,
                                    wildcard_additional_options,
                                ) => todo!(),
                                SelectItem::Wildcard(wildcard_additional_options) => todo!(),
                            }
                        }
                    }
                }
            }
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((select_items,)) = node_path.last_1_as::<Vec<SelectItem>>() {
            return select_items.iter().any(|select_item| {
                matches!(
                    self.node_types.get(&NodeKey::new(select_item)),
                    Some(Type::Value(Value::Eql(_)))
                )
            });
        }
        false
    }
}

fn cast_expr_to_jsonb(inner: Expr) -> Expr {
    Expr::Cast {
        kind: CastKind::DoubleColon,
        expr: Box::new(inner),
        data_type: DataType::JSONB,
        format: None,
    }
}
