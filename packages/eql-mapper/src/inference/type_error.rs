use std::sync::Arc;

use crate::{unifier::Type, SchemaError, ScopeError};

use super::unifier::EqlTraits;

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TypeError {
    #[error("SQL feature {} is not supported", _0)]
    UnsupportedSqlFeature(String),

    #[error("{}", _0)]
    InternalError(String),

    #[error("{}", _0)]
    Conflict(String),

    #[error("Type `{}` does not satisfy bounds `{}`", _0, _1)]
    UnsatisfiedBounds(Arc<Type>, EqlTraits),

    /// A second JSON traversal of a value that is already the result of one.
    ///
    /// An extracted entry is not a document — it has no `sv` array — so a
    /// further accessor selects nothing and the query silently returns NULL.
    /// A chain written in one expression is fused into a single path instead,
    /// so this is reached only when the chain is broken up such that the
    /// selectors cannot be composed: across a subquery boundary, a CTE, or a
    /// view.
    #[error(
        "cannot apply a JSON operator to the result of an encrypted JSON \
         operation: an extracted field is a single encrypted entry, not a \
         document, so there is nothing left to traverse. A multi-step path is \
         resolved against the whole document only by exact equality (`col -> 'a' \
         -> 'b' = $1`, or `<>`); anywhere else, select the one field you need"
    )]
    UnqueryableJsonExtraction,

    #[error("unified type contains unresolved type variable: {}", _0)]
    Incomplete(String),

    #[error("{}", _0)]
    Expected(String),

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

    #[error("Cannot parse placeholder syntax '{}'", _0)]
    ParamSyntax(String),

    #[error("{}", _0)]
    TypeSignature(String),
}
