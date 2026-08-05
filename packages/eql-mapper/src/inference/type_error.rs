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
    /// A chain written in ONE expression is collapsed into a single accessor
    /// carrying the composed path, so this is reached only when the chain is
    /// broken up such that the path cannot be composed: across a subquery
    /// boundary, a CTE, or a view.
    ///
    /// That case is not merely unimplemented. The type of an extracted value does
    /// not carry the path that produced it, and the root document is not even in
    /// scope on the far side of the boundary, so there is nothing to root a
    /// composed path at.
    #[error(
        "cannot apply a JSON operator to the result of an encrypted JSON \
         operation: an extracted field is a single encrypted entry, not a \
         document, so there is nothing left to traverse. Write the whole path in \
         one expression (`col -> 'a' -> 'b'`), which is resolved against the \
         document as a single path, rather than splitting it across a subquery, \
         CTE or view"
    )]
    UnqueryableJsonExtraction,

    /// A step of an encrypted JSON accessor chain that is neither a literal nor a
    /// placeholder.
    ///
    /// A chain collapses to ONE accessor keyed on the composed path, so every
    /// step has to be resolvable to path text by the time the proxy encrypts the
    /// selector. A step the proxy cannot resolve — a column reference, a function
    /// call — would be dropped from the statement along with the rest of the
    /// chain, silently changing which field the query reads.
    #[error(
        "every step of an encrypted JSON path must be a literal or a placeholder: \
         the whole chain is collapsed into one keyed path, so a step computed by \
         the database cannot contribute to it"
    )]
    UncomposableJsonPath,

    /// One placeholder used as the selector of two chains with different paths.
    ///
    /// The path a selector operand keys is recorded against the param it arrives
    /// in, because at Bind time the param number is all the proxy has. Two
    /// different paths for one param cannot both be honoured, and picking either
    /// answers the other occurrence from the wrong field.
    #[error(
        "placeholder ${0} is used as an encrypted JSON selector for two different \
         paths; give each path its own placeholder"
    )]
    AmbiguousJsonSelectorPath(u16),

    #[error("unified type contains unresolved type variable: {}", _0)]
    Incomplete(String),

    #[error(
        "the type of value `{}` was never constrained during type inference; \
         refusing to assume it is native",
        _0
    )]
    UnresolvedValue(String),

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

    /// A referenced table has a column declared with an encrypted-column type
    /// this build cannot map (see [`crate::ColumnKind::UnmappableEncrypted`]).
    ///
    /// This is a refusal, not a coverage gap: serving the statement would mean
    /// treating the column as plaintext.
    #[error(
        "Column `{}.{}` is declared as `{}`, which this build of CipherStash Proxy cannot encrypt or decrypt. Statements referencing `{}` are refused so that plaintext is never written to it. Migrate the column to an EQL v3 domain type.",
        table,
        column,
        column_type,
        table
    )]
    UnmappableEncryptedColumn {
        table: String,
        column: String,
        column_type: String,
    },
}

impl TypeError {
    /// Returns `(table, column, column_type)` when this error is the
    /// unmappable-encrypted-column refusal.
    ///
    /// Callers use this to tell a refusal apart from an ordinary type-check
    /// failure, because the two must not be handled the same way: a type-check
    /// failure may fall back to passthrough, a refusal never may.
    pub fn as_unmappable_encrypted_column(&self) -> Option<(&str, &str, &str)> {
        match self {
            TypeError::UnmappableEncryptedColumn {
                table,
                column,
                column_type,
            } => Some((table, column, column_type)),
            _ => None,
        }
    }
}
