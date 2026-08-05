use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{
    DuplicateTreatment, Function, FunctionArg, FunctionArgExpr, FunctionArgumentClause,
    FunctionArguments,
};

use crate::{
    get_sql_function,
    inference::infer_type::InferType,
    unifier::{Type, Value},
    EqlTrait, TypeError, TypeInferencer,
};

/// Looks up the function signature.
///
/// If a signature is found it means that function is handled as an EQL special case and is type checked accordingly.
///
/// If a signature is not found then all function args and its return type are unified as native.
///
/// The window specification of a window function (`OVER (...)` or the named
/// `WINDOW w AS (...)` it refers to) is constrained by the [`WindowSpec`]
/// impl in `window_spec.rs`, which sees the spec node wherever it is written.
///
/// [`WindowSpec`]: sqltk::parser::ast::WindowSpec
#[trace_infer]
impl<'ast> InferType<'ast, Function> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, function: &'ast Function) -> Result<(), TypeError> {
        if !matches!(function.parameters, FunctionArguments::None) {
            return Err(TypeError::UnsupportedSqlFeature(
                "Clickhouse-style function parameters".into(),
            ));
        }

        if let FunctionArguments::List(list) = &function.args {
            // `DISTINCT` dedupes the argument values by equality, so every
            // argument needs an equality term. Without the bound the dedup runs
            // on the raw jsonb payload — whose ciphertext is randomised per
            // row — so every value looks distinct and `count(DISTINCT enc)`
            // silently returns the row count.
            if list.duplicate_treatment == Some(DuplicateTreatment::Distinct) {
                for arg in &list.args {
                    if let FunctionArg::Unnamed(FunctionArgExpr::Expr(expr))
                    | FunctionArg::Named {
                        arg: FunctionArgExpr::Expr(expr),
                        ..
                    }
                    | FunctionArg::ExprNamed {
                        arg: FunctionArgExpr::Expr(expr),
                        ..
                    } = arg
                    {
                        self.unify_node_with_bound(expr, EqlTrait::Eq)?;
                    }
                }
            }

            // An `ORDER BY` inside the argument list (`array_agg(x ORDER BY
            // enc)`) sorts the values fed to the aggregate, so every key needs
            // an ordering term — otherwise the aggregate is built in raw
            // ciphertext order, which differs on every insert.
            for clause in &list.clauses {
                if let FunctionArgumentClause::OrderBy(order_by_exprs) = clause {
                    for order_by_expr in order_by_exprs {
                        self.unify_node_with_bound(&order_by_expr.expr, EqlTrait::Ord)?;
                    }
                }
            }
        }

        // An ordered-set aggregate (`percentile_disc(...) WITHIN GROUP (ORDER
        // BY key)`) computes its result *from* the sort key, so rewriting an
        // encrypted key to its ordering term would hand the client the term
        // itself — an opaque, undecryptable value. Reject an encrypted key
        // explicitly; anything else is unified as native, exactly like the
        // arguments of a function outside the EQL registry, so nothing can
        // slip through as an unresolved variable.
        for order_by_expr in &function.within_group {
            let ty = self.get_node_type(&order_by_expr.expr);
            let ty = ty.follow_tvars(&self.unifier.borrow());
            if matches!(&*ty, Type::Value(Value::Eql(_))) {
                return Err(TypeError::UnsupportedSqlFeature(
                    "WITHIN GROUP (ORDER BY ...) on an encrypted column".into(),
                ));
            }

            self.unify_node_with_type(&order_by_expr.expr, Type::native())?;
        }

        get_sql_function(&function.name).apply_constraints(self, function)
    }
}
