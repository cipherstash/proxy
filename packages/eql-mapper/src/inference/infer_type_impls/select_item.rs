use sqlparser::ast::SelectItem;

use crate::{
    inference::{type_error::TypeError, InferType},
    TypeInferencer,
};

impl<'ast> InferType<'ast, SelectItem> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, select_item: &'ast SelectItem) -> Result<(), TypeError> {
        let scope = self.scope_tracker.borrow();

        let ty = match select_item {
            SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => {
                self.get_type_var(expr)
            }
            SelectItem::QualifiedWildcard(object_name, _) => {
                scope.resolve_qualified_wildcard(&object_name.0)?
            }
            SelectItem::Wildcard(_) => scope.resolve_wildcard()?,
        };

        self.unify_node_with_type(select_item, ty)?;

        Ok(())
    }
}
