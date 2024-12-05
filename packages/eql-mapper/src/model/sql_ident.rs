use std::fmt::Debug;
use std::hash::{Hash, Hasher};

use derive_more::Display;
use sqlparser::ast::Ident;

/// The `SqlIdent` type wraps an [`Ident`] and defines a [`PartialEq`] implementation that respects the case-insensitive
/// versus case-sensitive comparison rules for SQL identifiers depending on whether the identifier is quoted or not.
///
/// All identifiers loaded from a database [`crate::model::schema::Schema`] are modelled as quoted.  Identifiers in SQL
/// statements are either quoted or unquoted: never canonical.
///
/// For an "official" explanation of how SQL identifiers work (at least with respect to Postgres), see
/// [https://www.postgresql.org/docs/14/sql-syntax-lexical.html#SQL-SYNTAX-IDENTIFIERS].
///
/// SQL is wild, hey!
#[derive(Debug, Clone, Eq, PartialOrd, Ord, Display)]
#[display("{}", _0)]
pub struct SqlIdent<'a>(pub &'a Ident);

impl<'a> SqlIdent<'a> {
    fn zipped(&'a self, b: &'a Self) -> impl Iterator<Item = (char, char)> + 'a {
        self.0.value.chars().zip(b.0.value.chars())
    }
}

// This manual Hash implementation is required to prevent a clippy error:
// "error: you are deriving `Hash` but have implemented `PartialEq` explicitly"
impl Hash for SqlIdent<'_> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state)
    }
}

/// Implements conditionally case-insensitive comparison without heap allocation.
impl PartialEq for SqlIdent<'_> {
    fn eq(&self, other: &Self) -> bool {
        if self.0.value.len() != other.0.value.len() {
            return false;
        }

        match (self.0.quote_style, other.0.quote_style) {
            (None, None) => self
                .zipped(other)
                .all(|(a, b)| a.to_lowercase().zip(b.to_lowercase()).all(|(a, b)| a == b)),

            (None, Some(_)) => self
                .zipped(other)
                .all(|(a, b)| a.to_lowercase().all(|a| a == b)),

            (Some(_), None) => self
                .zipped(other)
                .all(|(a, b)| b.to_lowercase().all(|b| a == b)),

            (Some(_), Some(_)) => self.0.value == other.0.value,
        }
    }
}

impl<'a> From<&'a Ident> for SqlIdent<'a> {
    fn from(ident: &'a Ident) -> Self {
        Self(ident)
    }
}
