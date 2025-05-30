use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::Delete;

use crate::{
    inference::unifier::Type,
    inference::{InferType, TypeError},
    TypeInferencer,
};

#[trace_infer]
impl<'ast> InferType<'ast, Delete> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, delete: &'ast Delete) -> Result<(), TypeError> {
        let Delete { returning, .. } = delete;

        match returning {
            Some(select_items) => {
                self.unify_nodes(delete, select_items)?;
            }

            None => {
                self.unify_node_with_type(delete, Type::empty_projection())?;
            }
        }

        Ok(())
    }
}
