use sqlparser::ast::BinaryOperator;

use crate::{SchemaError, ScopeError};

#[derive(PartialEq, Eq, Clone, Debug, thiserror::Error)]
pub enum TypeError {
    #[error("Unknown table-column")]
    UnknownTableColumn,

    #[error("Function call has one or more unmet constraints when called with encrypted term")]
    UnmetFunctionCallConstraint,

    #[error("SQL feature {} is not supported", _0)]
    UnsupportedSqlFeature(String),

    #[error("{}", _0)]
    UnableToInferType(String),

    #[error("{}", _0)]
    UnsupportedType(String),

    #[error("{}", _0)]
    InternalError(String),

    #[error("{}", _0)]
    Conflict(String),

    #[error("unified type contains unresolved type variables")]
    Incomplete,

    #[error("{}", _0)]
    UnsupportedOp(BinaryOperator),

    #[error("Uncomparable type: {}", _0)]
    UncomparableType(String),

    #[error("Unequatable type: {}", _0)]
    UnequatableType(String),

    #[error("Type does not support LIKE/ILIKE {}", _0)]
    LikeNotSupported(String),

    #[error("Unsatisfied constraint {}", _0)]
    Unsatisfied(String),

    #[error("Unsupported statement variant: only SELECT, INSERT, UPDATE & DELETE are supported")]
    UnsupportedStatementVariant,

    #[error("Unknown table '{}'", _0)]
    UnknownTable(String),

    #[error("Unsatisfied contraint on non-finalized type")]
    NotFinal,

    #[error(transparent)]
    ScopeError(#[from] ScopeError),

    #[error(transparent)]
    SchemaError(#[from] SchemaError),

    #[error("{}", _0)]
    Expected(String),
}
