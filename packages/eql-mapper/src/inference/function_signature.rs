use sqlparser::ast::ObjectName;

use super::{Def, Type, TypeVar};

/// A description of the signature of a function.
///
/// Note that this is NOT a `Check` implementation. The `InferType` impl for `Function`
/// will turn this description into a set of contraints.
// TODO: remove this when function checking is re-enabled.
#[allow(dead_code)]
pub enum FunctionSig {
    OneArg(Option<Type>, ReturnType),
    ZeroOrManyArgs,
}

/// A description of a constraint on the return type of a function call.
pub enum ReturnType {
    /// Return type must be `ExprType::Native`
    Native,

    // Return type must be same type as the sole argument type.
    IsSameAsArgType,
}

pub fn metadata_for_function(sqlfn: &ObjectName) -> FunctionSig {
    let name = &sqlfn
        .0
        .last()
        .expect("a function name (parse was successful so this should not fail)");

    let name = name.value.to_lowercase();

    if &name == "min" || &name == "max" {
        return FunctionSig::OneArg(
            Some(Type::new(Def::Var(TypeVar::Fresh))),
            ReturnType::IsSameAsArgType,
        );
    }

    if &name == "count" {
        return FunctionSig::OneArg(None, ReturnType::Native);
    }

    FunctionSig::ZeroOrManyArgs
}
