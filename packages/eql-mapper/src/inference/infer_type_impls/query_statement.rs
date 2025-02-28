use sqlparser::ast::Query;

use crate::{inference::InferType, inference::TypeError, TypeInferencer};

impl<'ast> InferType<'ast, Query> for TypeInferencer {
    fn infer_exit(&mut self, query: &'ast Query) -> Result<(), TypeError> {
        let Query { body, .. } = query;

        self.unify(self.get_type(query), self.get_type(&**body))?;

        Ok(())
    }
}
