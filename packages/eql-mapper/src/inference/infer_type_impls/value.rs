use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{Value, ValueWithSpan};

use crate::{
    inference::{type_error::TypeError, InferType},
    TypeInferencer,
};

/// Handles type inference for [`Value`] nodes - which include *literals* and *placeholders* (SQL param usages).
///
/// Params are tracked by name and unified against all other usages of themselves.
#[trace_infer]
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

/// Value nodes are wrapped in a [`ValueWithSpan`], so arrange for the type of the [`Value`] to propagate to the parent.
#[trace_infer]
impl<'ast> InferType<'ast, ValueWithSpan> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, value_with_span: &'ast ValueWithSpan) -> Result<(), TypeError> {
        self.unify(
            self.get_node_type(value_with_span),
            self.get_node_type(&value_with_span.value),
        )?;

        Ok(())
    }
}
