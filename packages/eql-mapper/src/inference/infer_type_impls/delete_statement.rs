use sqlparser::ast::Delete;

use crate::{
    inference::unifier::Type,
    inference::{InferType, TypeError},
    TypeInferencer,
};

impl<'ast> InferType<'ast, Delete> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, delete: &'ast Delete) -> Result<(), TypeError> {
        let Delete { returning, .. } = delete;

        match returning {
            Some(select_items) => {
                self.unify(self.get_type(delete), self.get_type(select_items))?;
            }

            None => {
                self.unify(self.get_type(delete), Type::empty())?;
            }
        }

        Ok(())
    }
}
