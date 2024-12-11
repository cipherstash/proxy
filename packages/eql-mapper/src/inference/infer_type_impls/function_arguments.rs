use sqlparser::ast::{FunctionArg, FunctionArgExpr, FunctionArgumentList, FunctionArguments};

use crate::{
    inference::type_error::TypeError, inference::unifier::Type, inference::InferType,
    TypeInferencer,
};

impl<'ast> InferType<'ast, FunctionArguments> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, function_args: &'ast FunctionArguments) -> Result<(), TypeError> {
        let this_type = self.get_type(function_args);

        match function_args {
            FunctionArguments::None => {
                self.unify_and_log(function_args, this_type, Type::empty())?;
            }

            FunctionArguments::Subquery(query) => {
                self.unify_and_log(function_args, this_type, self.get_type(&**query))?;
            }

            FunctionArguments::List(FunctionArgumentList { args, .. }) => {
                // FIXME: we need a tuple-like type to model function arguments.
                if args.is_empty() {
                    self.unify_and_log(function_args, this_type, Type::empty())?;
                } else {
                    for arg in args.iter() {
                        match arg {
                            FunctionArg::Named { arg, .. } | FunctionArg::Unnamed(arg) => {
                                let arg_type = self.get_type(arg);

                                if let FunctionArgExpr::Expr(expr) = arg {
                                    self.unify_and_log(arg, arg_type, self.get_type(expr))?;
                                }
                            }
                        }
                    }
                }
            }
        };

        Ok(())
    }
}
