use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{Function, FunctionArguments};

use crate::{
    get_type_signature_for_special_cased_sql_function, inference::infer_type::InferType,
    CompoundIdent, FunctionSig, TypeError, TypeInferencer,
};

/// Looks up the function signature.
///
/// If a signature is found it means that function is handled as an EQL special case and is type checked accordingly.
///
/// If a signature is not found then all function args and its return type are unified as native.
#[trace_infer]
impl<'ast> InferType<'ast, Function> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, function: &'ast Function) -> Result<(), TypeError> {
        if !matches!(function.parameters, FunctionArguments::None) {
            return Err(TypeError::UnsupportedSqlFeature(
                "Clickhouse-style function parameters".into(),
            ));
        }

        let Function { name, args, .. } = function;
        let fn_name = CompoundIdent::from(&name.0);

        match get_type_signature_for_special_cased_sql_function(&fn_name, args) {
            Some(sig) => {
                sig.instantiate(&*self).apply_constraints(self, function)?;
            }
            None => {
                FunctionSig::instantiate_native(function).apply_constraints(self, function)?;
            }
        }

        Ok(())
    }
}
