use std::sync::Arc;

use crate::{unifier::Type, SchemaError, ScopeError};

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

    #[error("Expected param count to be {}, but got {}", _0, _1)]
    ParamCount(usize, usize),

    #[error("{}", _0)]
    FunctionCall(String),

    #[error("{}", _0)]
    ScopeError(#[from] ScopeError),

    #[error("{}", _0)]
    SchemaError(#[from] SchemaError),

    #[error(
        "Cannot unify node types for nodes:\n 1. node: {} type: {}\n 2. node: {} type: {}\n error: {}",
        _0,
        _1,
        _2,
        _3,
        _4
    )]
    OnNodes(String, Arc<Type>, String, Arc<Type>, String),
}
