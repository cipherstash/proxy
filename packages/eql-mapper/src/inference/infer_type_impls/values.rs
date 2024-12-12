use sqlparser::ast::Values;
use tracing::info;

use crate::{
    inference::type_error::TypeError, inference::unifier::Type, inference::InferType,
    TypeInferencer,
};

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
            .map(|val| self.get_type(val))
            .collect::<Vec<_>>();

        info!("WAT 0");
        for row in values.rows.iter() {
            for (idx, val) in row.iter().enumerate() {
                self.unify(self.get_type(val), column_types[idx].clone())?;
            }
        }

        info!("WAT 0.1");

        self.unify_and_log(
            values,
            self.get_type(values),
            Type::projection(
                &column_types
                    .iter()
                    .map(|ty| (ty.clone(), None))
                    .collect::<Vec<_>>(),
            ),
        )?;

        info!("WAT 1");

        Ok(())
    }
}
