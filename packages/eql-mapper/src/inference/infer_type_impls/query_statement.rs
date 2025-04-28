use eql_mapper_macros::trace_infer;
use sqltk_parser::ast::Query;

use crate::{
    inference::{InferType, TypeError},
    TypeInferencer,
};

#[trace_infer]
impl<'ast> InferType<'ast, Query> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, query: &'ast Query) -> Result<(), TypeError> {
        let Query { body, .. } = query;

        self.unify_nodes(query, &**body)?;

        Ok(())
    }
}
