#![allow(unused)]

use sqlparser::ast::{Function, FunctionArguments};

use crate::{
    inference::InferType,
    inference::{metadata_for_function, type_error::TypeError, FunctionSig, ReturnType},
    TypeInferencer,
};

impl<'ast> InferType<'ast, Function> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, function: &'ast Function) -> Result<(), TypeError> {
        if !matches!(function.parameters, FunctionArguments::None) {
            return Err(TypeError::UnsupportedSqlFeature(
                "Clickhouse-style function parameters".into(),
            ));
        }

        let this_type = self.get_type(function);

        let Function { name, args, .. } = function;

        let args_type = self.get_type(args);

        // match metadata_for_function(self.new_tvar(), name) {
        //     FunctionSig::OneArg(expr_type_constraint, ReturnType::IsSameAsArgType)
        //         if args_type_hole.len() == 1 =>
        //     {
        //         let arg_type_hole = &mut args_type_hole[0];
        //         if let Some(expr_type_constraint) = expr_type_constraint {
        //             arg_type_hole.constrain(expr_type_constraint)?;
        //         }
        //         this_type_hole.unify(arg_type_hole)?;
        //     }

        //     FunctionSig::OneArg(expr_type_constraint, ReturnType::Native)
        //         if args_type_hole.len() == 1 =>
        //     {
        //         let arg_type_hole = &mut args_type_hole[0];
        //         if let Some(expr_type_constraint) = expr_type_constraint {
        //             arg_type_hole.constrain(expr_type_constraint)?;
        //         }
        //         this_type_hole.finalize(ConcreteType::Native)?;
        //     }

        //     FunctionSig::OneArg(_, _) => {
        //         return Err(TypeError::Conflict(format!(
        //             "expected function {} to have 1 argument",
        //             name
        //         )))
        //     }

        //     FunctionSig::ZeroOrManyArgs => {
        //         for mut arg in args_type_hole {
        //             arg.finalize(ConcreteType::Native)?;
        //         }
        //         this_type_hole.finalize(ConcreteType::Native)?;
        //     }
        // }

        Ok(())
    }
}
