use std::cmp::Ordering;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};

use derive_more::Display;
use sqltk_parser::ast::Ident;

/// `SqlIdent` wraps an [`Ident`] (or `&Ident`) and defines a [`PartialEq`] implementation that respects the
/// case-insensitive versus case-sensitive comparison rules for SQL identifiers depending on whether the identifier is
/// quoted or not.
///
/// For an "official" explanation of how SQL identifiers work (at least with respect to Postgres), see
/// [https://www.postgresql.org/docs/14/sql-syntax-lexical.html#SQL-SYNTAX-IDENTIFIERS].
///
/// SQL is wild, hey!
#[derive(Debug, Clone, Display)]
pub struct SqlIdent<T>(pub T);

impl<T> PartialOrd for SqlIdent<T>
where
    Self: PartialEq,
    T: PartialOrd,
{
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        self.0.partial_cmp(&other.0)
    }
}

impl<T> Ord for SqlIdent<T>
where
    Self: PartialEq,
    T: Eq + Ord,
{
    fn cmp(&self, other: &Self) -> Ordering {
        self.0.cmp(&other.0)
    }
}

impl<T> Eq for SqlIdent<T>
where
    Self: PartialEq,
    T: Eq,
{
}

impl PartialEq for SqlIdent<Ident> {
    fn eq(&self, other: &Self) -> bool {
        SqlIdent(&self.0) == SqlIdent(&other.0)
    }
}

impl PartialEq<SqlIdent<Ident>> for SqlIdent<&Ident> {
    fn eq(&self, other: &SqlIdent<Ident>) -> bool {
        self == &SqlIdent(&other.0)
    }
}

impl<'a> PartialEq<SqlIdent<&'a Ident>> for SqlIdent<Ident> {
    fn eq(&self, other: &SqlIdent<&'a Ident>) -> bool {
        SqlIdent(&self.0) == SqlIdent(other.0)
    }
}

/// Implements conditionally case-insensitive comparison without heap allocation.
impl PartialEq for SqlIdent<&Ident> {
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

impl<'a> SqlIdent<&'a Ident> {
    fn zipped(&self, b: &Self) -> impl Iterator<Item = (char, char)> + 'a {
        self.0.value.chars().zip(b.0.value.chars())
    }

    pub fn as_deref(&self) -> Self {
        SqlIdent(self.0)
    }
}

impl SqlIdent<Ident> {
    pub fn as_deref(&self) -> SqlIdent<&Ident> {
        SqlIdent(&self.0)
    }
}

// This manual Hash implementation is required to prevent a clippy error:
// "error: you are deriving `Hash` but have implemented `PartialEq` explicitly"
impl<T> Hash for SqlIdent<T>
where
    T: Hash,
{
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state)
    }
}

impl<'a> From<&'a Ident> for SqlIdent<&'a Ident> {
    fn from(ident: &'a Ident) -> Self {
        Self(ident)
    }
}

impl From<Ident> for SqlIdent<Ident> {
    fn from(ident: Ident) -> Self {
        Self(ident)
    }
}

#[cfg(test)]
mod test {
    use sqltk_parser::ast::Ident;

    use crate::SqlIdent;

    macro_rules! id {
        ($name:ident) => {
            Ident::from(stringify!($name))
        };

        ($name:expr) => {
            Ident::with_quote('"', $name)
        };
    }

    #[test]
    fn owned_versus_borrowed() {
        assert_eq!(SqlIdent(&id!(email)), SqlIdent(id!(email)));
        assert_eq!(SqlIdent(id!(eMaIl)), SqlIdent(&id!(email)));
        assert_eq!(SqlIdent(&id!(email)), SqlIdent(&id!(email)));
        assert_eq!(SqlIdent(id!(eMaIl)), SqlIdent(id!(email)));
    }

    #[test]
    fn unquoted_unquoted() {
        assert_eq!(SqlIdent(id!(email)), SqlIdent(id!(email)));
        assert_eq!(SqlIdent(id!(eMaIl)), SqlIdent(id!(email)));
        assert_ne!(SqlIdent(id!(age)), SqlIdent(id!(email)));
    }

    #[test]
    fn quoted_quoted() {
        assert_eq!(SqlIdent(id!("email")), SqlIdent(id!("email")));
        assert_ne!(SqlIdent(id!("Email")), SqlIdent(id!("email")));
    }

    #[test]
    fn quoted_unquoted() {
        assert_eq!(SqlIdent(id!("email")), SqlIdent(id!(email)));
        assert_ne!(SqlIdent(id!("Email")), SqlIdent(id!(email)));
        assert_ne!(SqlIdent(id!(email)), SqlIdent(id!("Email")));
    }
}
