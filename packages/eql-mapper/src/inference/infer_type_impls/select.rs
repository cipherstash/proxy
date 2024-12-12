use sqlparser::ast::Select;

use crate::{
    inference::{type_error::TypeError, InferType},
    TypeInferencer,
};

impl<'ast> InferType<'ast, Select> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, select: &'ast Select) -> Result<(), TypeError> {
        self.unify_and_log(
            select,
            self.get_type(select),
            self.get_type(&select.projection),
        )?;

        Ok(())
    }
}
