use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{Function, FunctionArguments, Ident};

use crate::{
    get_sql_function,
    inference::{infer_type::InferType, sql_types::CompoundIdent},
    TypeError, TypeInferencer,
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

        let fully_qualified_fn_name = if function.name.0.len() == 1 {
            CompoundIdent::from(&vec![Ident::new("pg_catalog"), function.name.0[0].clone()])
        } else {
            CompoundIdent::from(&function.name.0)
        };

        get_sql_function(&fully_qualified_fn_name).apply_constraints(self, function)
    }
}
