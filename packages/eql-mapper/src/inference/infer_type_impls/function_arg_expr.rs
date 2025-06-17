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
            // COUNT(*) is the only function in SQL (that I can find) that accepts a wildcard as an argument.  And it is
            // *not* an expression - it is special case syntax that means "count all rows".  If we see this syntax, we
            // resolve the FunctionArgExpr type as Native.
            FunctionArgExpr::QualifiedWildcard(_) | FunctionArgExpr::Wildcard => {
                self.unify(farg_expr_ty, Type::any_native())?;
            }
        };

        Ok(())
    }
}
