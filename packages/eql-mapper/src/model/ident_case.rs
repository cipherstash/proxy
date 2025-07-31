use std::cmp::Ordering;
use std::fmt::Debug;
use std::hash::{Hash, Hasher};

use derive_more::Display;
use sqltk::parser::ast::{Ident, ObjectName, ObjectNamePart};

/// `IdentCase` wraps an [`Ident`] or [`ObjectName`] and defines a [`PartialEq`] implementation that respects the
/// case-insensitive versus case-sensitive comparison rules for SQL identifiers depending on whether the identifiers are
/// quoted or not.
///
/// Implements conditionally case-insensitive comparison without heap allocation.
///
/// For an "official" explanation of how SQL identifiers work (at least with respect to Postgres), see
/// [<https://www.postgresql.org/docs/14/sql-syntax-lexical.html#SQL-SYNTAX-IDENTIFIERS>].
///
/// SQL is wild, hey!
#[derive(Debug, Clone, Display)]
pub struct IdentCase<T>(pub T);

impl<'a> IdentCase<&'a Ident> {
    fn zipped(&self, b: &Self) -> impl Iterator<Item = (char, char)> + 'a {
        self.0.value.chars().zip(b.0.value.chars())
    }
}

impl IdentCase<ObjectName> {
    pub(crate) fn starts_with(&self, other: &IdentCase<Ident>) -> bool {
        let ObjectNamePart::Identifier(first) = &self.0 .0[0];
        IdentCase(first) == IdentCase(&other.0)
    }
}

impl Eq for IdentCase<&'_ Ident> {}
impl Eq for IdentCase<Ident> {}

impl PartialEq for IdentCase<&'_ Ident> {
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

impl PartialEq for IdentCase<Ident> {
    fn eq(&self, other: &Self) -> bool {
        IdentCase(&self.0) == IdentCase(&other.0)
    }
}

impl Eq for IdentCase<&'_ ObjectName> {}
impl Eq for IdentCase<ObjectName> {}

impl PartialEq for IdentCase<&'_ ObjectName> {
    fn eq(&self, other: &Self) -> bool {
        if self.0 .0.len() != other.0 .0.len() {
            return false;
        }

        for (mine, theirs) in self.0 .0.iter().zip(other.0 .0.iter()) {
            if mine == theirs {
                continue;
            }

            match (mine.as_ident(), theirs.as_ident()) {
                (None, None) => {
                    continue;
                }
                (None, Some(_)) => {
                    return false;
                }
                (Some(_), None) => {
                    return false;
                }
                (Some(mine), Some(theirs)) => {
                    if IdentCase(mine) != IdentCase(theirs) {
                        return false;
                    }
                }
            }
        }

        true
    }
}

impl PartialEq for IdentCase<ObjectName> {
    fn eq(&self, other: &Self) -> bool {
        IdentCase(&self.0) == IdentCase(&other.0)
    }
}

impl PartialEq<IdentCase<Ident>> for IdentCase<&Ident> {
    fn eq(&self, other: &IdentCase<Ident>) -> bool {
        self == &IdentCase(&other.0)
    }
}

impl PartialEq<IdentCase<&Ident>> for IdentCase<Ident> {
    fn eq(&self, other: &IdentCase<&Ident>) -> bool {
        &IdentCase(&self.0) == other
    }
}

impl PartialEq<IdentCase<ObjectName>> for IdentCase<&ObjectName> {
    fn eq(&self, other: &IdentCase<ObjectName>) -> bool {
        self == &IdentCase(&other.0)
    }
}

impl PartialEq<IdentCase<&ObjectName>> for IdentCase<ObjectName> {
    fn eq(&self, other: &IdentCase<&ObjectName>) -> bool {
        &IdentCase(&self.0) == other
    }
}

impl Ord for IdentCase<&'_ Ident> {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.0.value.len().cmp(&other.0.value.len()) {
            Ordering::Less => Ordering::Less,

            Ordering::Greater => Ordering::Greater,

            Ordering::Equal => match (self.0.quote_style, other.0.quote_style) {
                (None, None) => {
                    for (a, b) in self.zipped(other) {
                        let compared: Ordering = a.to_lowercase().zip(b.to_lowercase()).fold(
                            Ordering::Equal,
                            |acc, (lhs, rhs)| {
                                if acc != Ordering::Equal {
                                    return acc;
                                }

                                lhs.cmp(&rhs)
                            },
                        );

                        if compared != Ordering::Equal {
                            return compared;
                        }
                    }

                    Ordering::Equal
                }

                (None, Some(_)) => {
                    for (a, b) in self.zipped(other) {
                        let compared: Ordering =
                            a.to_lowercase().fold(Ordering::Equal, |acc, lhs| {
                                if acc != Ordering::Equal {
                                    return acc;
                                }

                                lhs.cmp(&b)
                            });

                        if compared != Ordering::Equal {
                            return compared;
                        }
                    }

                    Ordering::Equal
                }

                (Some(_), None) => {
                    for (a, b) in self.zipped(other) {
                        let compared: Ordering =
                            b.to_lowercase().fold(Ordering::Equal, |acc, rhs| {
                                if acc != Ordering::Equal {
                                    return acc;
                                }

                                a.cmp(&rhs)
                            });

                        if compared != Ordering::Equal {
                            return compared;
                        }
                    }

                    Ordering::Equal
                }

                (Some(_), Some(_)) => self.0.value.cmp(&other.0.value),
            },
        }
    }
}

impl PartialOrd for IdentCase<&'_ Ident> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl PartialOrd<IdentCase<Ident>> for IdentCase<&Ident> {
    fn partial_cmp(&self, other: &IdentCase<sqltk::parser::ast::Ident>) -> Option<Ordering> {
        self.partial_cmp(&IdentCase(&other.0))
    }
}

impl PartialOrd<IdentCase<&Ident>> for IdentCase<Ident> {
    fn partial_cmp(&self, other: &IdentCase<&sqltk::parser::ast::Ident>) -> Option<Ordering> {
        IdentCase(&self.0).partial_cmp(other)
    }
}

impl Ord for IdentCase<&'_ ObjectName> {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.0 .0.len().cmp(&other.0 .0.len()) {
            Ordering::Less => Ordering::Less,
            Ordering::Greater => Ordering::Greater,
            Ordering::Equal => {
                for (ObjectNamePart::Identifier(mine), ObjectNamePart::Identifier(theirs)) in
                    self.0 .0.iter().zip(other.0 .0.iter())
                {
                    match IdentCase(mine).cmp(&IdentCase(theirs)) {
                        Ordering::Equal => {
                            continue;
                        }
                        Ordering::Less => return Ordering::Less,
                        Ordering::Greater => return Ordering::Greater,
                    }
                }

                Ordering::Equal
            }
        }
    }
}

impl PartialOrd for IdentCase<&'_ ObjectName> {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

// This Hash implementation (and the following) one is required in order to be consistent with PartialEq.
impl Hash for IdentCase<&Ident> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        match self.0.quote_style {
            Some(ch) => {
                state.write_u8(1);
                state.write_u32(ch as u32);
                state.write(self.0.value.as_bytes());
            }
            None => {
                state.write_u8(0);
                for ch in self.0.value.chars().flat_map(|ch| ch.to_lowercase()) {
                    state.write_u32(ch as u32);
                }
            }
        }
    }
}

impl Hash for IdentCase<Ident> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        IdentCase(&self.0).hash(state)
    }
}

// This Hash implementation (and the following) one is required in order to be consistent with PartialEq.
impl Hash for IdentCase<&ObjectName> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for ObjectNamePart::Identifier(ident) in self.0 .0.iter() {
            IdentCase(ident).hash(state);
        }
    }
}

impl Hash for IdentCase<ObjectName> {
    fn hash<H: Hasher>(&self, state: &mut H) {
        IdentCase(&self.0).hash(state)
    }
}

impl<'a> From<&'a Ident> for IdentCase<&'a Ident> {
    fn from(ident: &'a Ident) -> Self {
        Self(ident)
    }
}

impl From<Ident> for IdentCase<Ident> {
    fn from(ident: Ident) -> Self {
        Self(ident)
    }
}

impl<'a> From<&'a ObjectName> for IdentCase<&'a ObjectName> {
    fn from(object_name: &'a ObjectName) -> Self {
        Self(object_name)
    }
}

impl From<ObjectName> for IdentCase<ObjectName> {
    fn from(object_name: ObjectName) -> Self {
        Self(object_name)
    }
}

#[cfg(test)]
mod test {
    use sqltk::parser::ast::{Ident, ObjectName, ObjectNamePart};

    use crate::IdentCase;

    macro_rules! id {
        ($name:ident) => {
            Ident::from(stringify!($name))
        };

        ($name:literal) => {
            Ident::with_quote('"', $name)
        };
    }

    macro_rules! objname {
        ($first:ident . $second:ident) => {
            ObjectName(vec![
                ObjectNamePart::Identifier(Ident::from(stringify!($first))),
                ObjectNamePart::Identifier(Ident::from(stringify!($second))),
            ])
        };

        ($first:literal . $second:literal) => {
            ObjectName(vec![
                ObjectNamePart::Identifier(Ident::with_quote('"', $first)),
                ObjectNamePart::Identifier(Ident::with_quote('"', $second)),
            ])
        };
    }

    #[test]
    fn owned_versus_borrowed() {
        assert_eq!(IdentCase(&id!(email)), IdentCase(id!(email)));
        assert_eq!(IdentCase(id!(eMaIl)), IdentCase(&id!(email)));
        assert_eq!(IdentCase(&id!(email)), IdentCase(&id!(email)));
        assert_eq!(IdentCase(id!(eMaIl)), IdentCase(id!(email)));

        assert_eq!(
            IdentCase(&objname!(customer.email)),
            IdentCase(objname!(customer.email))
        );
        assert_eq!(
            IdentCase(objname!(customer.eMaIl)),
            IdentCase(&objname!(customer.email))
        );
        assert_eq!(
            IdentCase(&objname!(customer.email)),
            IdentCase(&objname!(customer.email))
        );
        assert_eq!(
            IdentCase(objname!(customer.eMaIl)),
            IdentCase(objname!(customer.email))
        );
    }

    #[test]
    fn unquoted_unquoted() {
        assert_eq!(IdentCase(id!(email)), IdentCase(id!(email)));
        assert_eq!(IdentCase(id!(eMaIl)), IdentCase(id!(email)));
        assert_ne!(IdentCase(id!(age)), IdentCase(id!(email)));

        assert_eq!(
            IdentCase(objname!(customer.email)),
            IdentCase(objname!(customer.email))
        );
        assert_eq!(
            IdentCase(objname!(customer.eMaIl)),
            IdentCase(objname!(customer.email))
        );
        assert_ne!(
            IdentCase(objname!(customer.age)),
            IdentCase(objname!(customer.email))
        );
        assert_ne!(
            IdentCase(objname!(person.email)),
            IdentCase(objname!(customer.email))
        );
    }

    #[test]
    fn quoted_quoted() {
        assert_eq!(IdentCase(id!("email")), IdentCase(id!("email")));
        assert_ne!(IdentCase(id!("Email")), IdentCase(id!("email")));

        assert_eq!(
            IdentCase(objname!("customer"."email")),
            IdentCase(objname!("customer"."email"))
        );
        assert_ne!(
            IdentCase(objname!("customer"."Email")),
            IdentCase(objname!("customer"."email"))
        );
    }

    #[test]
    fn quoted_unquoted() {
        assert_eq!(IdentCase(id!("email")), IdentCase(id!(email)));
        assert_ne!(IdentCase(id!("Email")), IdentCase(id!(email)));
        assert_ne!(IdentCase(id!(email)), IdentCase(id!("Email")));

        assert_eq!(
            IdentCase(objname!("customer"."email")),
            IdentCase(objname!(customer.email))
        );
        assert_ne!(
            IdentCase(objname!("customer"."Email")),
            IdentCase(objname!(customer.email))
        );
        assert_ne!(
            IdentCase(objname!(customer.email)),
            IdentCase(objname!("customer"."Email"))
        );
    }
}
