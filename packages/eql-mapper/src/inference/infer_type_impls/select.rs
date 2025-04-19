use eql_mapper_macros::trace_infer;
use sqlparser::ast::Select;

use crate::{
    inference::{type_error::TypeError, InferType},
    TypeInferencer,
};

#[trace_infer]
impl<'ast> InferType<'ast, Select> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, select: &'ast Select) -> Result<(), TypeError> {
        self.unify_nodes(select, &select.projection)?;

        Ok(())
    }
}
