use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{SelectItem, SelectItemQualifiedWildcardKind};

use crate::{
    inference::{type_error::TypeError, InferType},
    TypeInferencer,
};

#[trace_infer]
impl<'ast> InferType<'ast, SelectItem> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, select_item: &'ast SelectItem) -> Result<(), TypeError> {
        match select_item {
            SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => {
                self.unify_node_with_type(select_item, self.get_node_type(expr))?;
            }
            SelectItem::QualifiedWildcard(
                SelectItemQualifiedWildcardKind::ObjectName(object_name),
                _,
            ) => {
                self.unify_node_with_type(
                    select_item,
                    self.resolve_qualified_wildcard(object_name)?,
                )?;
            }
            SelectItem::QualifiedWildcard(SelectItemQualifiedWildcardKind::Expr(_), _) => {
                return Err(TypeError::UnsupportedSqlFeature(
                    "qualified wildcards on arbitrary expressions".into(),
                ))
            }
            SelectItem::Wildcard(_) => {
                self.unify_node_with_type(select_item, self.resolve_wildcard()?)?;
            }
        }

        Ok(())
    }
}
