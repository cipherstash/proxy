use crate::{
    inference::{type_error::TypeError, unifier::Type, InferType},
    TypeInferencer,
};
use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::Values;

#[trace_infer]
impl<'ast> InferType<'ast, Values> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, values: &'ast Values) -> Result<(), TypeError> {
        if values.rows.is_empty() {
            return Err(TypeError::InternalError(
                "Empty VALUES expression".to_string(),
            ));
        }

        let col_count = values.rows.first().unwrap().len();

        if !values.rows.iter().all(|row| row.len() == col_count) {
            return Err(TypeError::InternalError(
                "Mixed row lengths in VALUES expression".to_string(),
            ));
        }

        let column_types = &values.rows[0]
            .iter()
            .map(|val| self.get_node_type(val))
            .collect::<Vec<_>>();

        for row in values.rows.iter() {
            for (idx, expr) in row.iter().enumerate() {
                self.unify(self.get_node_type(expr), column_types[idx].clone())?;
            }
        }

        self.unify_node_with_type(
            values,
            Type::projection(
                &column_types
                    .iter()
                    .map(|ty| (ty.clone(), None))
                    .collect::<Vec<_>>(),
            ),
        )?;

        Ok(())
    }
}
