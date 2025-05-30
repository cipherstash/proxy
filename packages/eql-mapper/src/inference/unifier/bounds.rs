use std::{cmp::Ordering, collections::BTreeSet, sync::Arc};

use derive_more::derive::Display;

use crate::EqlTraitImpls;

use super::{Constructor, EqlTerm, EqlValue, Projection, Type, Value, Var};

/// Represents the supported operations on an EQL type
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Display, Hash)]
pub enum EqlTrait {
    #[display("Eq")]
    Eq,
    #[display("Ord")]
    Ord,
    #[display("Bloom")]
    Bloom,
    #[display("Json")]
    Json,
    #[display("JsonAccessor")]
    JsonQuery(Arc<Type>),
}

impl EqlTrait {
    pub(crate) fn implied(&self) -> Vec<EqlTrait> {
        if let EqlTrait::Ord = self {
            vec![EqlTrait::Eq]
        } else {
            vec![]
        }
    }
}

/// Represents the set of "traits" implemented by a [`Type`].
///
/// EQL types _and_ native types are tested against the bounds, but the trick is that native types *always* satisfy all
/// of the bounds (we let the database do its job - it will shout loudly when an expression has been used incorrectly).
///
/// EQL types _must_ implement every individually required bound. This information will eventually let us produce good
/// error messages, but implemented bounds are exposed to consumers [`crate::TypeCheckedStatement`] in order to inform
/// how to encrypt literals and params whether for storage or querying.
///
/// [`Bounds`] values always successfully unify into a superset of traits.
#[derive(Debug, PartialEq, Eq, PartialOrd, Clone, Default, Hash)]
pub enum Bounds {
    /// No trait bounds
    #[default]
    None,
    /// All trait bounds (Native types always pass trait bounds checks and this is how they're handled)
    All,
    /// These specific traits are defined as bounds.
    Explicit(BTreeSet<EqlTrait>),
}

impl From<Vec<EqlTrait>> for Bounds {
    fn from(eql_traits: Vec<EqlTrait>) -> Self {
        let mut bounds = Bounds::None;

        for eql_trait in eql_traits {
            bounds = bounds.union(&Bounds::from(eql_trait))
        }

        bounds
    }
}

impl From<EqlTrait> for Bounds {
    fn from(eql_trait: EqlTrait) -> Self {
        let mut bounds = BTreeSet::new();

        // Deals with traits that imply another: e.g. Ord implies Eq.
        for implied_eql_trait in eql_trait.implied() {
            bounds.insert(implied_eql_trait);
        }

        bounds.insert(eql_trait);

        Self::Explicit(bounds)
    }
}

impl Bounds {
    pub(crate) fn none() -> Self {
        Self::default()
    }

    pub(crate) fn union(&self, other: &Self) -> Self {
        match (self, other) {
            (Bounds::None, Bounds::None) => Bounds::None,
            (Bounds::None, Bounds::All) => Bounds::All,
            (Bounds::None, Bounds::Explicit(bounds)) => Bounds::Explicit(bounds.clone()),
            (Bounds::All, Bounds::None) => Bounds::All,
            (Bounds::All, Bounds::All) => Bounds::All,
            (Bounds::All, Bounds::Explicit(_)) => Bounds::All,
            (Bounds::Explicit(bounds), Bounds::None) => Bounds::Explicit(bounds.clone()),
            (Bounds::Explicit(_), Bounds::All) => Bounds::All,
            (Bounds::Explicit(bounds_a), Bounds::Explicit(bounds_b)) => {
                Bounds::Explicit(BTreeSet::from_iter(bounds_a.union(bounds_b).cloned()))
            }
        }
    }

    pub(crate) fn intersection(&self, other: &Self) -> Self {
        match (self, other) {
            (Bounds::None, _) => Bounds::None,
            (_, Bounds::None) => Bounds::None,
            (Bounds::All, Bounds::All) => Bounds::All,
            (Bounds::All, Bounds::Explicit(eql_traits)) => Bounds::Explicit(eql_traits.clone()),
            (Bounds::Explicit(eql_traits), Bounds::All) => Bounds::Explicit(eql_traits.clone()),
            (Bounds::Explicit(a), Bounds::Explicit(b)) => Bounds::Explicit(
                a.intersection(b).cloned().collect()
            ),
        }
    }

    pub(crate) fn difference(&self, other: &Self) -> Self {
        match (self, other) {
            (Bounds::None, _) => Bounds::None,
            (Bounds::All, Bounds::All) => Bounds::None,
            (Bounds::All, _) => Bounds::All,
            (Bounds::Explicit(_), Bounds::None) => self.clone(),
            (Bounds::Explicit(_), Bounds::All) => Bounds::All,
            (Bounds::Explicit(a), Bounds::Explicit(b)) => {
                Bounds::Explicit(a.difference(b).cloned().collect())
            }
        }
    }
}

impl std::fmt::Display for Bounds {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Bounds::None => Ok(()),
            Bounds::All => f.write_str("ALL"),
            Bounds::Explicit(bounds) if bounds.len() == 1 => bounds.first().unwrap().fmt(f),
            Bounds::Explicit(bounds) => {
                let mut is_first = true;
                for bound in bounds {
                    if is_first {
                        bound.fmt(f)?;
                        is_first = false;
                    } else {
                        f.write_fmt(format_args!("+ {}", bound))?;
                    }
                }
                Ok(())
            }
        }
    }
}

impl Ord for Bounds {
    fn cmp(&self, other: &Self) -> Ordering {
        match (self, other) {
            (Bounds::None, Bounds::None) => Ordering::Equal,
            (Bounds::None, Bounds::All) => Ordering::Less,
            (Bounds::None, Bounds::Explicit(bounds)) if bounds.len() == 0 => Ordering::Equal,
            (Bounds::None, Bounds::Explicit(_)) => Ordering::Less,
            (Bounds::All, Bounds::None) => Ordering::Greater,
            (Bounds::All, Bounds::All) => Ordering::Equal,
            (Bounds::All, Bounds::Explicit(_)) => Ordering::Greater,
            (Bounds::Explicit(bounds), Bounds::None) if bounds.len() == 0 => Ordering::Equal,
            (Bounds::Explicit(_), Bounds::None) => Ordering::Greater,
            (Bounds::Explicit(_), Bounds::All) => Ordering::Less,
            (Bounds::Explicit(bounds_a), Bounds::Explicit(bounds_b)) if bounds_a == bounds_b => {
                Ordering::Equal
            }
            (Bounds::Explicit(bounds_a), Bounds::Explicit(bounds_b))
                if bounds_a.len() > bounds_b.len() =>
            {
                Ordering::Greater
            }
            (Bounds::Explicit(bounds_a), Bounds::Explicit(bounds_b))
                if bounds_a.len() < bounds_b.len() =>
            {
                Ordering::Less
            }
            (Bounds::Explicit(bounds_a), Bounds::Explicit(bounds_b)) => {
                for (a, b) in bounds_a.iter().zip(bounds_b) {
                    if a < b {
                        return Ordering::Less;
                    } else if a > b {
                        return Ordering::Greater;
                    }
                }
                Ordering::Equal
            }
        }
    }
}

impl Type {
    pub(crate) fn effective_bounds(&self) -> Bounds {
        match self {
            Type::Constructor(constructor) => constructor.effective_bounds(),
            Type::Var(Var(_, bounds)) => bounds.clone(),
        }
    }
}

impl Constructor {
    pub(crate) fn effective_bounds(&self) -> Bounds {
        match self {
            Constructor::Value(value) => value.effective_bounds(),
            Constructor::Projection(projection) => projection.effective_bounds(),
        }
    }
}

impl Value {
    pub(crate) fn effective_bounds(&self) -> Bounds {
        match self {
            Value::Eql(eql_term) => eql_term.effective_bounds(),
            Value::Native(_) => Bounds::All,
            Value::Array(ty) => ty.effective_bounds(),
        }
    }
}

impl Projection {
    pub(crate) fn effective_bounds(&self) -> Bounds {
        match self {
            Projection::WithColumns(cols) => {
                if let Some((first, rest)) = cols.0.split_first() {
                    let mut acc = first.ty.effective_bounds();
                    for col in rest {
                        acc = acc.intersection(&col.ty.effective_bounds())
                    }
                    return acc;
                }
                unreachable!("there is always at least one column in Projection::WithColumns")
            }
            Projection::Empty => Bounds::All,
        }
    }
}

impl EqlTerm {
    pub(crate) fn effective_bounds(&self) -> Bounds {
        match self {
            EqlTerm::Whole(eql_value) => eql_value.effective_bounds(),
            EqlTerm::Partial(_, bounds) => bounds.clone(),
            EqlTerm::FixedPartial(_, bounds) => bounds.clone(),
        }
    }
}

impl EqlValue {
    pub(crate) fn effective_bounds(&self) -> Bounds {
        Bounds::from(self.trait_impls())
    }
}

impl From<&EqlTraitImpls> for Bounds {
    fn from(impls: &EqlTraitImpls) -> Self {
        let mut bounds = Bounds::None;

        if impls.implements_eq() {
            bounds = bounds.union(&Bounds::from(EqlTrait::Eq));
        }

        if impls.implements_ord() {
            bounds = bounds.union(&Bounds::from(EqlTrait::Ord));
        }

        if impls.implements_bloom() {
            bounds = bounds.union(&Bounds::from(EqlTrait::Bloom));
        }

        if impls.implements_json() {
            bounds = bounds.union(&Bounds::from(EqlTrait::Json));
        }

        bounds
    }
}
