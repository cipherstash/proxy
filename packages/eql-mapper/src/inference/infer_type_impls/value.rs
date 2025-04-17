use sqlparser::ast::Value;

use crate::{
    inference::{type_error::TypeError, InferType}, TypeInferencer
};

/// Handles type inference for [`Value`] nodes - which include *literals* and *placeholders* (SQL param usages).
///
/// Params are tracked by name and unified against all other usages of themselves.
impl<'ast> InferType<'ast, Value> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, value: &'ast Value) -> Result<(), TypeError> {
        let value_ty = self.get_node_type(value);

        if let Value::Placeholder(param) = value {
            let reg = self.reg.borrow();
            // Check if we've seen this param already
            match reg.get_param(param) {
                Some(existing_param_ty) => {
                    drop(reg);
                    // Unify the node's type with the existing param type
                    self.unify(value_ty, existing_param_ty)?;
                }
                None => {
                    // We haven't seen the param before so set the current node's type to a fresh type variable and
                    // register the param with the same type.
                    drop(reg);
                    let mut reg = self.reg.borrow_mut();
                    reg.set_param(param, value_ty);
                }
            }
        }

        Ok(())
    }
}
