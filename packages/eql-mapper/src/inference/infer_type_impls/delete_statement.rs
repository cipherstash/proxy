use sqlparser::ast::Delete;

use crate::{
    inference::{InferType, Type, TypeError},
    TypeInferencer,
};

impl<'ast> InferType<'ast, Delete> for TypeInferencer {
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
