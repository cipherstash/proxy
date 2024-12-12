use sqlparser::ast::SelectItem;

use crate::{
    inference::{type_error::TypeError, unifier::Type, InferType},
    TypeInferencer,
};

impl<'ast> InferType<'ast, Vec<SelectItem>> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, select_items: &'ast Vec<SelectItem>) -> Result<(), TypeError> {
        let projections: Vec<_> = select_items
            .iter()
            .map(|select_item| self.get_type(select_item))
            .collect();

        self.unify(
            self.get_type(select_items),
            Type::flatten_projections(projections)?,
        )?;

        Ok(())
    }
}
