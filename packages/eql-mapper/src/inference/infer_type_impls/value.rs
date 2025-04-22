use sqltk_parser::ast::Value;

use crate::{
    inference::{type_error::TypeError, InferType},
    TypeInferencer,
};

/// Handles type inference for [`Value`] nodes - which include *literals* and *placeholders* (SQL param usages).
///
/// Params are tracked by name and unified against all other usages of themselves.
impl<'ast> InferType<'ast, Value> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, value: &'ast Value) -> Result<(), TypeError> {
        if let Value::Placeholder(param) = value {
            self.unify(self.get_node_type(value), self.get_param_type(param))?;
        } else {
            self.unify(self.get_node_type(value), self.fresh_tvar())?;
        }

        Ok(())
    }
}
