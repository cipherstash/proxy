use sqlparser::ast::{FunctionArg, FunctionArgExpr, FunctionArgumentList, FunctionArguments};

use crate::{
    inference::InferType,
    inference::{type_error::TypeError, Type},
    TypeInferencer,
};

impl<'ast> InferType<'ast, FunctionArguments> for TypeInferencer {
    fn infer_enter(&mut self, function_args: &'ast FunctionArguments) -> Result<(), TypeError> {
        let this_type = self.get_type(function_args);

        match function_args {
            FunctionArguments::None => {
                self.unify(this_type, Type::empty())?;
            }

            FunctionArguments::Subquery(query) => {
                self.unify(this_type, self.get_type(&**query))?;
            }

            FunctionArguments::List(FunctionArgumentList { args, .. }) => {
                for arg in args.iter() {
                    match arg {
                        FunctionArg::Named { arg, .. } | FunctionArg::Unnamed(arg) => {
                            let arg_type = self.get_type(arg);

                            if let FunctionArgExpr::Expr(expr) = arg {
                                self.unify(arg_type, self.get_type(expr))?;
                            }
                        }
                    }
                }
            }
        };

        Ok(())
    }

    fn infer_exit(&mut self, _: &'ast FunctionArguments) -> Result<(), TypeError> {
        Ok(())
    }
}
