use std::collections::HashSet;

use crate::{SchemaError, ScopeError};

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TypeError {
    #[error("SQL feature {} is not supported", _0)]
    UnsupportedSqlFeature(String),

    #[error("{}", _0)]
    InternalError(String),

    #[error("{}", _0)]
    Conflict(String),

    #[error("unified type contains unresolved type variable: {}", _0)]
    Incomplete(String),

    #[error("{}", _0)]
    Expected(String),

    #[error("One or more params failed to unify: {}", _0.iter().cloned().collect::<Vec<String>>().join(", "))]
    Params(HashSet<String>),

    #[error("Expected scalar type for param {} but got type {}", _0, _1)]
    NonScalarParam(String, String),

    #[error("Expected param count to be {}, but got {}", _0, _1)]
    ParamCount(usize, usize),

    #[error("{}", _0)]
    FunctionCall(String),

    #[error("{}", _0)]
    ScopeError(#[from] ScopeError),

    #[error("{}", _0)]
    SchemaError(#[from] SchemaError),
}
