use sqlparser::ast::Select;

use crate::{
    inference::{type_error::TypeError, unifier::Type, InferType},
    TypeInferencer,
};

impl<'ast> InferType<'ast, Select> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, select: &'ast Select) -> Result<(), TypeError> {
        let projections: Vec<_> = select
            .projection
            .iter()
            .map(|select_item| self.get_type(select_item))
            .collect();

        self.unify_and_log(
            select,
            self.get_type(select),
            Type::flatten_projections(projections)?,
        )?;

        Ok(())
    }
}
