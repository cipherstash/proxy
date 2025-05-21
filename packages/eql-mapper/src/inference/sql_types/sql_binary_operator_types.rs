use sqltk::parser::ast::{BinaryOperator, Expr};

use crate::{unifier::{Type, TypeArg, TypeEnv}, TypeError, TypeInferencer};

#[derive(Debug)]
pub(crate) struct ExplicitBinaryOpRule {
    #[allow(unused)]
    op: BinaryOperator,
    env: TypeEnv,
    lhs_type_arg: TypeArg,
    rhs_type_arg: TypeArg,
    return_type_arg: TypeArg,
}

/// A rule for determining how to apply typing rules to a SQL binary operator expression.
#[derive(Debug)]
pub(crate) enum SqlBinaryOp {
    /// An explicit predefined rule for handling EQL types in the expression.
    Explicit(&'static ExplicitBinaryOpRule),

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
                let instantiated = rule.env.instantiate(&mut inferencer.unifier.borrow_mut())?;
                inferencer.unify_node_with_type(lhs, instantiated[&rule.lhs_type_arg].clone())?;
                inferencer.unify_node_with_type(op_expr, instantiated[&rule.return_type_arg].clone())?;
                inferencer.unify_node_with_type(rhs, instantiated[&rule.rhs_type_arg].clone())?;
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

impl ExplicitBinaryOpRule {
    pub(crate) fn new(
        operator: BinaryOperator,
        type_env: TypeEnv,
        lhs_ty: TypeArg,
        rhs_ty: TypeArg,
        ret_ty: TypeArg,
    ) -> Self {
        assert!(type_env.contains_key(&lhs_ty));
        assert!(type_env.contains_key(&rhs_ty));
        assert!(type_env.contains_key(&ret_ty));

        Self {
            op: operator,
            lhs_type_arg: lhs_ty,
            rhs_type_arg: rhs_ty,
            return_type_arg: ret_ty,
            env: type_env,
        }
    }
}