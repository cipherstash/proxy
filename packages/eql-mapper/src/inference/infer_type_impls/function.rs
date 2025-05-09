use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{Function, FunctionArg, FunctionArgExpr, FunctionArguments, Ident};

use crate::{
    inference::{type_error::TypeError, InferType},
    unifier::Type,
    SqlIdent, TypeInferencer,
};

#[trace_infer]
impl<'ast> InferType<'ast, Function> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, function: &'ast Function) -> Result<(), TypeError> {
        if !matches!(function.parameters, FunctionArguments::None) {
            return Err(TypeError::UnsupportedSqlFeature(
                "Clickhouse-style function parameters".into(),
            ));
        }

        let Function { name, args, .. } = function;

        let fn_name: Vec<_> = name.0.iter().map(SqlIdent).collect();

        if fn_name == [SqlIdent(&Ident::new("min"))] || fn_name == [SqlIdent(&Ident::new("max"))] {
            // 1. There MUST be one unnamed argument (it CAN come from a subquery)
            // 2. The return type is the same as the argument type

            match args {
                FunctionArguments::None => {
                    return Err(TypeError::FunctionCall(format!(
                        "{} should be called with 1 argument, got 0",
                        fn_name.last().unwrap()
                    )))
                }

                FunctionArguments::Subquery(query) => {
                    // The query must return a single column projection which has the same type as the result of the
                    // call to min/max.
                    self.unify_node_with_type(
                        &**query,
                        Type::projection(&[(self.get_node_type(function), None)]),
                    )?;
                }

                FunctionArguments::List(args_list) => {
                    if args_list.args.len() == 1 {
                        match &args_list.args[0] {
                            FunctionArg::Named { .. } | FunctionArg::ExprNamed { .. } => {
                                return Err(TypeError::FunctionCall(format!(
                                    "{} cannot be called with named arguments",
                                    fn_name.last().unwrap(),
                                )))
                            }

                            FunctionArg::Unnamed(function_arg_expr) => match function_arg_expr {
                                FunctionArgExpr::Expr(expr) => {
                                    self.unify_nodes(function, expr)?;
                                }

                                FunctionArgExpr::QualifiedWildcard(_)
                                | FunctionArgExpr::Wildcard => {
                                    return Err(TypeError::FunctionCall(format!(
                                        "{} cannot be called with wildcard arguments",
                                        fn_name.last().unwrap(),
                                    )))
                                }
                            },
                        }
                    } else {
                        return Err(TypeError::FunctionCall(format!(
                            "{} should be called with 1 argument, got {}",
                            fn_name.last().unwrap(),
                            args_list.args.len()
                        )));
                    }
                }
            }
        } else {
            // All other functions: resolve to native
            // EQL values will be rejected in function calls
            self.unify_node_with_type(function, Type::any_native())?;

            match args {
                // Function called without any arguments.
                // Used for functions like `CURRENT_TIMESTAMP` that do not require parentheses ()
                // This is not the same as a function that has zero arguments (which would be an empty arg list)
                FunctionArguments::None => {}

                FunctionArguments::Subquery(query) => {
                    // The query must return a single column projection which has the same type as the result of the function
                    self.unify_node_with_type(
                        &**query,
                        Type::projection(&[(self.get_node_type(function), None)]),
                    )?;
                }

                FunctionArguments::List(args_list) => {
                    self.unify_node_with_type(function, Type::any_native())?;
                    for arg in &args_list.args {
                        match arg {
                            FunctionArg::ExprNamed {
                                name,
                                arg,
                                operator: _,
                            } => {
                                self.unify_node_with_type(name, Type::any_native())?;
                                match arg {
                                    FunctionArgExpr::Expr(expr) => {
                                        self.unify_node_with_type(expr, Type::any_native())?;
                                    }
                                    // Aggregate functions like COUNT(table.*)
                                    FunctionArgExpr::QualifiedWildcard(_) => {}
                                    // Aggregate functions like COUNT(*)
                                    FunctionArgExpr::Wildcard => {}
                                }
                            }
                            FunctionArg::Named { arg, .. } | FunctionArg::Unnamed(arg) => match arg
                            {
                                FunctionArgExpr::Expr(expr) => {
                                    self.unify_node_with_type(expr, Type::any_native())?;
                                }
                                // Aggregate functions like COUNT(table.*)
                                FunctionArgExpr::QualifiedWildcard(_) => {}
                                // Aggregate functions like COUNT(*)
                                FunctionArgExpr::Wildcard => {}
                            },
                        }
                    }
                }
            }
        }

        Ok(())
    }
}
