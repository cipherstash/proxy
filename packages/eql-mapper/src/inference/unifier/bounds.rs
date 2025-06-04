use derive_more::derive::Display;

use super::{
    Array,  Constructor, EqlTerm, EqlValue,  Projection, Type, Value,
    Var,
};

/// Represents the supported operations on an EQL type
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Display, Hash)]
pub enum EqlTrait {
    #[display("Eq")]
    Eq,
    #[display("Ord")]
    Ord,
    #[display("Bloom")]
    Bloom,
    #[display("Json")]
    Json,
    #[display("Containment")]
    Containment,
    #[display("JsonFieldAccess")]
    JsonFieldAccess,
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
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Default, Hash)]
pub struct EqlTraits {
    /// The column implements equality between its values using the `=` operator.
    pub eq: bool,

    /// The column implements comparison of its values using `>`, `>=`, `=`, `<=`, `<`.
    /// `ord` implies `eq`.
    pub ord: bool,

    /// The column implements textual substring search using `LIKE`.
    pub bloom: bool,

    /// The column implements [`EqlTrait::Containment`] & [`EqlTrait::JsonFieldAccess`]
    pub json: bool,

    /// The column implements containment checking (e.g. `@>` and `<@`)
    pub containment: bool,

    /// The column implements JSON field access (`->` etc)
    pub json_field_access: bool,
}

pub const ALL_TRAITS: EqlTraits = EqlTraits {
    eq: true,
    ord: true,
    bloom: true,
    json: true,
    containment: true,
    json_field_access: true,
};

impl From<EqlTrait> for EqlTraits {
    fn from(eql_trait: EqlTrait) -> Self {
        let mut traits = EqlTraits::default();
        traits.add_mut(eql_trait);
        traits
    }
}

impl FromIterator<EqlTrait> for EqlTraits {
    fn from_iter<T: IntoIterator<Item = EqlTrait>>(iter: T) -> Self {
        let mut traits = EqlTraits::default();
        for t in iter {
            traits.add_mut(t)
        }
        traits
    }
}

impl EqlTraits {
    pub(crate) fn none() -> Self {
        Self::default()
    }

    pub(crate) fn add_mut(&mut self, eql_trait: EqlTrait) {
        match eql_trait {
            EqlTrait::Eq => self.eq = true,
            // Ord implies Eq
            EqlTrait::Ord => {
                self.eq = true;
                self.ord = true;
            }
            EqlTrait::Bloom => self.bloom = true,
            EqlTrait::Json => {
                self.json = true;
                self.json_field_access = true;
                self.containment = true;
            }
            EqlTrait::Containment => self.containment = true,
            EqlTrait::JsonFieldAccess => self.json_field_access = true,
        }
    }

    pub(crate) fn union(&self, other: &Self) -> Self {
        EqlTraits {
            eq: self.eq || other.eq,
            ord: self.ord || other.ord,
            bloom: self.bloom || other.bloom,
            json: self.json || other.json,
            containment: self.containment || other.containment,
            json_field_access: self.json_field_access || other.json_field_access,
        }
    }

    pub(crate) fn intersection(&self, other: &Self) -> Self {
        EqlTraits {
            eq: self.eq && other.eq,
            ord: self.ord && other.ord,
            bloom: self.bloom && other.bloom,
            json: self.json && other.json,
            containment: self.containment && other.containment,
            json_field_access: self.json_field_access && other.json_field_access,
        }
    }

    pub(crate) fn difference(&self, other: &Self) -> Self {
        EqlTraits {
            eq: self.eq ^ other.eq,
            ord: self.ord ^ other.ord,
            bloom: self.bloom ^ other.bloom,
            json: self.json ^ other.json,
            containment: self.containment ^ other.containment,
            json_field_access: self.json_field_access ^ other.json_field_access,
        }
    }
}

impl std::fmt::Display for EqlTraits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const EQ: &'static str = "Eq";
        const ORD: &'static str = "Ord";
        const BLOOM: &'static str = "Bloom";
        const CONTAINMENT: &'static str = "Containment";
        const JSON_FIELD_ACCESS: &'static str = "JsonFieldAccess";

        let mut traits: Vec<&'static str> = Vec::new();
        if self.eq {
            traits.push(EQ)
        }
        if self.ord {
            traits.push(ORD)
        }
        if self.bloom {
            traits.push(BLOOM)
        }
        if self.containment {
            traits.push(CONTAINMENT)
        }
        if self.json_field_access {
            traits.push(JSON_FIELD_ACCESS)
        }

        f.write_str(&traits.join("+"))?;

        Ok(())
    }
}

impl Type {
    pub(crate) fn effective_bounds(&self) -> EqlTraits {
        match self {
            Type::Constructor(constructor) => constructor.effective_bounds(),
            Type::Var(Var(_, bounds)) => bounds.clone(),
            Type::Associated(associated_type) => associated_type.associated.effective_bounds()
        }
    }
}

impl Constructor {
    pub(crate) fn effective_bounds(&self) -> EqlTraits {
        match self {
            Constructor::Value(value) => value.effective_bounds(),
            Constructor::Projection(projection) => projection.effective_bounds(),
        }
    }
}

impl Value {
    pub(crate) fn effective_bounds(&self) -> EqlTraits {
        match self {
            Value::Eql(eql_term) => eql_term.effective_bounds(),
            Value::Native(_) => ALL_TRAITS, // 💪
            Value::Array(ty) => ty.effective_bounds(),
        }
    }
}

impl Array {
    pub(crate) fn effective_bounds(&self) -> EqlTraits {
        let Array(element_ty) = self;
        element_ty.effective_bounds()
    }
}

impl Projection {
    pub(crate) fn effective_bounds(&self) -> EqlTraits {
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
            Projection::Empty => ALL_TRAITS,
        }
    }
}

impl EqlTerm {
    pub(crate) fn effective_bounds(&self) -> EqlTraits {
        match self {
            EqlTerm::Full(eql_value) => eql_value.effective_bounds(),
            EqlTerm::Partial(_, bounds) => bounds.clone(),
            EqlTerm::JsonFieldAccessor(_) => EqlTraits::none(),
        }
    }
}

impl EqlValue {
    pub(crate) fn effective_bounds(&self) -> EqlTraits {
        self.trait_impls()
    }
}
