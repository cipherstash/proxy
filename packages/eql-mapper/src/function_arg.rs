use sqltk::parser::ast::{Expr, FunctionArg, FunctionArgExpr};

pub(crate) fn function_arg_expr(arg: &FunctionArg) -> &FunctionArgExpr {
    match arg {
        FunctionArg::Named { arg, .. } => arg,
        FunctionArg::ExprNamed { arg, .. } => arg,
        FunctionArg::Unnamed(arg) => arg,
    }
}

pub(crate) fn function_arg_value(arg: &FunctionArg) -> Option<&Expr> {
    match function_arg_expr(arg) {
        FunctionArgExpr::Expr(expr) => Some(expr),
        FunctionArgExpr::QualifiedWildcard(_) | FunctionArgExpr::Wildcard => None,
    }
}

pub(crate) fn function_arg_value_mut(arg: &mut FunctionArg) -> Option<&mut Expr> {
    let arg = match arg {
        FunctionArg::Named { arg, .. } => arg,
        FunctionArg::ExprNamed { arg, .. } => arg,
        FunctionArg::Unnamed(arg) => arg,
    };
    match arg {
        FunctionArgExpr::Expr(expr) => Some(expr),
        FunctionArgExpr::QualifiedWildcard(_) | FunctionArgExpr::Wildcard => None,
    }
}
