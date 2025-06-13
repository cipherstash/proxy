use sqltk::parser::ast::Expr;

use crate::{
    unifier::{BinaryOpDecl, Type},
    TypeError, TypeInferencer,
};

/// A rule for determining how to apply typing rules to a SQL binary operator expression.
#[derive(Debug)]
pub(crate) enum SqlBinaryOp {
    /// An explicit predefined rule for handling EQL types in the expression.
    Explicit(&'static BinaryOpDecl),

    /// The fallback rule for when there is no explicit rule for a given operator.  This rule will force the left and
    /// right expressions of the operator and its return value to resolve to [`Type::native()`].
    Fallback,
}

impl SqlBinaryOp {
    pub(crate) fn apply_constraints<'ast>(
        &self,
        inferencer: &mut TypeInferencer<'ast>,
        lhs: &'ast Expr,
        rhs: &'ast Expr,
        return_val: &'ast Expr,
    ) -> Result<(), TypeError> {
        match self {
            SqlBinaryOp::Explicit(rule) => {
                let lhs_ty = inferencer.get_node_type(lhs);
                let rhs_ty = inferencer.get_node_type(rhs);
                let ret_ty = inferencer.get_node_type(return_val);

                rule.inner.apply(
                    &mut inferencer.unifier.borrow_mut(),
                    &[lhs_ty, rhs_ty],
                    ret_ty,
                )?;
            }

            SqlBinaryOp::Fallback => {
                inferencer.unify_node_with_type(lhs, Type::native())?;
                inferencer.unify_node_with_type(rhs, Type::native())?;
                inferencer.unify_node_with_type(return_val, Type::native())?;
            }
        }

        Ok(())
    }
}
