use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::FunctionArgExpr;

use crate::{inference::infer_type::InferType, TypeError, TypeInferencer};

#[trace_infer]
impl<'ast> InferType<'ast, FunctionArgExpr> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, farg_expr: &'ast FunctionArgExpr) -> Result<(), TypeError> {
        let farg_expr_ty = self.get_node_type(farg_expr);
        match farg_expr {
            FunctionArgExpr::Expr(expr) => {
                self.unify(farg_expr_ty, self.get_node_type(expr))?;
            }
            FunctionArgExpr::QualifiedWildcard(qualified) => {
                self.unify(farg_expr_ty, self.resolve_qualified_wildcard(&qualified.0)?)?;
            }
            FunctionArgExpr::Wildcard => {
                self.unify(farg_expr_ty, self.resolve_wildcard()?)?;
            }
        };

        Ok(())
    }
}
