use sqlparser::ast::Value;

use crate::{
    inference::{type_error::TypeError, InferType},
    TypeInferencer,
};

/// Handles type inference for [`Value`] nodes - which include *literals* and *placeholders* (SQL param usages).
///
/// Params are tracked by name and unified against all other usages of themselves.
impl<'ast> InferType<'ast, Value> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, value: &'ast Value) -> Result<(), TypeError> {
        let value_tid = self.unify_node_with_type(value, self.fresh_tvar())?;

        if let Value::Placeholder(param) = value {
            let reg = self.reg.borrow();
            // Check if we've seen this param already
            match reg.get_param(param) {
                Some((existing_param_tid, _)) => {
                    drop(reg);
                    // Unify the node's type with the existing param type
                    self.unifier
                        .borrow_mut()
                        .unify(value_tid, existing_param_tid)?;
                }
                None => {
                    // We haven't seen the param before so set the current node's type to a fresh type variable and
                    // register the param with the same type.
                    drop(reg);
                    let ty = self.get_type_by_tid(value_tid);
                    let mut reg = self.reg.borrow_mut();
                    reg.set_param(param, ty);
                }
            }
        }

        Ok(())
    }
}
