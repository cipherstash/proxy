use std::{collections::HashMap, sync::Arc};

use sqltk::parser::ast::{self, CastKind, DataType, Query, Select, SelectItem, SetExpr, Statement};
use sqltk::parser::ast::{Expr, WildcardAdditionalOptions};
use sqltk::{AsNodeKey, NodeKey, NodePath, Visitable};

use crate::{EqlMapperError, Projection, Type, Value};

use super::TransformationRule;

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
            if let Some((_, _, _, _, original_select_items)) =
                node_path.last_5_as::<Statement, Query, SetExpr, Select, Vec<SelectItem>>()
            {
                if let Some(select_items) = target_node.downcast_mut::<Vec<SelectItem>>() {
                    let mut new_select_items: Vec<SelectItem> =
                        Vec::with_capacity(select_items.len());

                    for (original_select_item, select_item) in
                        original_select_items.iter().zip(select_items.iter_mut())
                    {
                        let mut detached: SelectItem = std::mem::replace(
                            select_item,
                            SelectItem::Wildcard(WildcardAdditionalOptions::default()),
                        );

                        if matches!(
                            self.node_types.get(&NodeKey::new(&*original_select_item)),
                            Some(Type::Value(Value::Eql(_)))
                        ) {
                            match &mut detached {
                                SelectItem::UnnamedExpr(expr)
                                | SelectItem::ExprWithAlias { expr, alias: _ } => {
                                    *expr = cast_expr_to_jsonb(std::mem::replace(
                                        expr,
                                        Expr::Value(ast::Value::Null),
                                    ));
                                    new_select_items.push(detached);
                                }
                                SelectItem::Wildcard(_) => {
                                    if let Some(Type::Projection(projection)) =
                                        self.node_types.get(&NodeKey::new(select_item))
                                    {
                                        match projection {
                                            Projection::WithColumns(cols) => {
                                                for col in cols {
                                                    match &col.alias {
                                                        Some(alias) => new_select_items.push(
                                                            SelectItem::UnnamedExpr(
                                                                Expr::Identifier(alias.clone()),
                                                            ),
                                                        ),
                                                        None => {
                                                            panic!("Dammit cannot handle this case you muppets")
                                                        }
                                                    }
                                                }
                                            }
                                            Projection::Empty => {}
                                        }
                                    }
                                }
                                SelectItem::QualifiedWildcard(object_name, _) => todo!(),
                            }
                        } else {
                            new_select_items.push(detached);
                        }
                    }
                    *select_items = new_select_items;
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
