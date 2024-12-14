use sqlparser::ast::Query;

use crate::{inference::InferType, inference::TypeError, TypeInferencer};

impl<'ast> InferType<'ast, Query> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, query: &'ast Query) -> Result<(), TypeError> {
        let Query { body, .. } = query;

        self.unify_nodes(query, &**body)?;

        Ok(())
    }
}
