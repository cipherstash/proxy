use sqltk_parser::ast::SelectItem;

use crate::{
    inference::{type_error::TypeError, InferType},
    TypeInferencer,
};

impl<'ast> InferType<'ast, SelectItem> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, select_item: &'ast SelectItem) -> Result<(), TypeError> {
        match select_item {
            SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => {
                self.unify_node_with_type(select_item, self.get_node_type(expr))?;
            }
            SelectItem::QualifiedWildcard(object_name, _) => {
                self.unify_node_with_type(
                    select_item,
                    self.resolve_qualified_wildcard(&object_name.0)?,
                )?;
            }
            SelectItem::Wildcard(_) => {
                self.unify_node_with_type(select_item, self.resolve_wildcard()?)?;
            }
        }

        Ok(())
    }
}
