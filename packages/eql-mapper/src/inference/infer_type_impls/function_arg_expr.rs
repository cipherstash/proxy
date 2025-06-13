use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::FunctionArgExpr;

use crate::{inference::infer_type::InferType, unifier::Type, TypeError, TypeInferencer};

#[trace_infer]
impl<'ast> InferType<'ast, FunctionArgExpr> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, farg_expr: &'ast FunctionArgExpr) -> Result<(), TypeError> {
        let farg_expr_ty = self.get_node_type(farg_expr);

        match farg_expr {
            FunctionArgExpr::Expr(expr) => {
                self.unify(farg_expr_ty, self.get_node_type(expr))?;
            }
            // `COUNT(*)` is a special case in SQL.  The `*` is NOT an expression - which would normally expand into a
            // projection. `COUNT(*)` merely means "count all rows". As such, we should not attempt to resolve it as
            // anything other than Native.
            FunctionArgExpr::QualifiedWildcard(_) | FunctionArgExpr::Wildcard => {
                self.unify(farg_expr_ty, Type::native())?;
            }
        };

        Ok(())
    }
}
