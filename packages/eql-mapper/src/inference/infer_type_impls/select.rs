use sqltk_parser::ast::Select;

use crate::{
    inference::{type_error::TypeError, InferType},
    TypeInferencer,
};

impl<'ast> InferType<'ast, Select> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, select: &'ast Select) -> Result<(), TypeError> {
        self.unify_nodes(select, &select.projection)?;

        Ok(())
    }
}
