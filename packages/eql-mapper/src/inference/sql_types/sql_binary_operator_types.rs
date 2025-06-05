use sqltk::parser::ast::Expr;

use crate::{
    unifier::{BinaryOpSpec, Type},
    TypeError, TypeInferencer,
};

/// A rule for determining how to apply typing rules to a SQL binary operator expression.
#[derive(Debug)]
pub(crate) enum SqlBinaryOp {
    /// An explicit predefined rule for handling EQL types in the expression.
    Explicit(&'static BinaryOpSpec),

    /// The fallback rule for when there is no explicit rule for a given operator.  This rule will force the left and
    /// right expressions of the operator and its return value to resolve to [`Type::native()`].
    Fallback,
}

impl SqlBinaryOp {
    pub(crate) fn apply_constraints<'ast>(
        &self,
        inferencer: &mut TypeInferencer<'ast>,
        lhs: &'ast Expr,
        op_expr: &'ast Expr,
        rhs: &'ast Expr,
    ) -> Result<(), TypeError> {
        match self {
            SqlBinaryOp::Explicit(rule) => {
                let lhs_ty = inferencer.get_node_type(lhs);
                let ret_ty = inferencer.get_node_type(op_expr);
                let rhs_ty = inferencer.get_node_type(rhs);

                rule.inner.init(
                    &mut inferencer.unifier.borrow_mut(),
                    &[lhs_ty, rhs_ty],
                    ret_ty
                )?;
            }

            SqlBinaryOp::Fallback => {
                inferencer.unify_node_with_type(lhs, Type::native())?;
                inferencer.unify_node_with_type(op_expr, Type::native())?;
                inferencer.unify_node_with_type(rhs, Type::native())?;
            }
        }

        Ok(())
    }
}
