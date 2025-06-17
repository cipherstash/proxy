use std::sync::Arc;

use derive_more::derive::{Deref, Display};

use crate::{unifier::{AssociatedTypeSelector, SetOf}, TypeError};

use super::{Array, Constructor, EqlTerm, EqlValue, Projection, Type, Value, Var};

/// Represents the supported operations on an EQL type
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Display, Hash)]
pub enum EqlTrait {
    #[display("Eq")]
    Eq,
    #[display("Ord")]
    Ord,
    #[display("TokenMatch")]
    TokenMatch,
    #[display("JsonLike")]
    JsonLike,
    #[display("Contain")]
    Contain,
}

#[derive(Debug, Deref)]
pub(crate) struct EqlTraitAssociatedTypes(#[deref] pub(crate) &'static [&'static str]);

const ASSOC_TYPES_EQ: &EqlTraitAssociatedTypes = &EqlTraitAssociatedTypes(&["Only"]);

const ASSOC_TYPES_ORD: &EqlTraitAssociatedTypes = &EqlTraitAssociatedTypes(&["Only"]);

const ASSOC_TYPES_TOKEN_MATCH: &EqlTraitAssociatedTypes = &EqlTraitAssociatedTypes(&["Tokenized"]);

const ASSOC_TYPES_JSON_LIKE: &EqlTraitAssociatedTypes =
    &EqlTraitAssociatedTypes(&["Path", "Accessor"]);

const ASSOC_TYPES_CONTAIN: &EqlTraitAssociatedTypes = &EqlTraitAssociatedTypes(&["Only"]);

impl EqlTrait {
    pub(crate) const fn associated_type_names(&self) -> &'static EqlTraitAssociatedTypes {
        match self {
            EqlTrait::Eq => ASSOC_TYPES_EQ,
            EqlTrait::Ord => ASSOC_TYPES_ORD,
            EqlTrait::TokenMatch => ASSOC_TYPES_TOKEN_MATCH,
            EqlTrait::JsonLike => ASSOC_TYPES_JSON_LIKE,
            EqlTrait::Contain => ASSOC_TYPES_CONTAIN,
        }
    }

    pub(crate) fn has_associated_type(&self, assoc_type_name: &str) -> bool {
        self.associated_type_names().contains(&assoc_type_name)
    }

    pub(crate) fn resolve_associated_type(
        &self,
        ty: Arc<Type>,
        selector: &AssociatedTypeSelector,
    ) -> Result<Arc<Type>, TypeError> {
        ty.clone()
            .must_implement(&EqlTraits::from(selector.eql_trait))?;

        match &*ty {
            // Native satisfies all associated type bounds
            Type::Constructor(Constructor::Value(Value::Native(_))) => {
                match (self, selector.type_name) {
                    (EqlTrait::Eq, "Only")
                    | (EqlTrait::Ord, "Only")
                    | (EqlTrait::TokenMatch, "Tokenized")
                    | (EqlTrait::JsonLike, "Accessor")
                    | (EqlTrait::JsonLike, "Path")
                    | (EqlTrait::Contain, "Only") => Ok(ty.clone()),
                    (_, unknown_associated_type) => Err(TypeError::InternalError(format!(
                        "Unknown associated type {}::{}",
                        self, unknown_associated_type
                    ))),
                }
            }
            Type::Constructor(Constructor::Value(Value::Eql(EqlTerm::Full(eql_col))))
            | Type::Constructor(Constructor::Value(Value::Eql(EqlTerm::Partial(eql_col, _)))) => {
                match (self, selector.type_name) {
                    (EqlTrait::Eq, "Only") => {
                        Ok(Arc::new(Type::Constructor(Constructor::Value(Value::Eql(
                            EqlTerm::Partial(eql_col.clone(), EqlTraits::from(EqlTrait::Eq)),
                        )))))
                    }
                    (EqlTrait::Ord, "Only") => {
                        Ok(Arc::new(Type::Constructor(Constructor::Value(Value::Eql(
                            EqlTerm::Partial(eql_col.clone(), EqlTraits::from(EqlTrait::Ord)),
                        )))))
                    }
                    (EqlTrait::TokenMatch, "Tokenized") => Ok(Arc::new(Type::Constructor(
                        Constructor::Value(Value::Eql(EqlTerm::Tokenized(eql_col.clone()))),
                    ))),
                    (EqlTrait::JsonLike, "Accessor") => {
                        Ok(Arc::new(Type::Constructor(Constructor::Value(
                            Value::Eql(EqlTerm::JsonAccessor(eql_col.clone())),
                        ))))
                    }
                    (EqlTrait::JsonLike, "Path") => Ok(Arc::new(Type::Constructor(
                        Constructor::Value(Value::Eql(EqlTerm::JsonPath(eql_col.clone()))),
                    ))),
                    (EqlTrait::Contain, "Only") => {
                        Ok(Arc::new(Type::Constructor(Constructor::Value(Value::Eql(
                            EqlTerm::Partial(eql_col.clone(), EqlTraits::from(EqlTrait::Contain)),
                        )))))
                    }
                    (_, unknown_associated_type) => Err(TypeError::InternalError(format!(
                        "Unknown associated type {}::{}",
                        self, unknown_associated_type
                    ))),
                }
            }
            _ => Err(TypeError::InternalError(format!(
                "associated type can only be resolved on Value::Native or Value::Eql types; got {ty}",
            ))),
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
/// [`BoundsDecl`] values always successfully unify into a superset of traits.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Default, Hash)]
pub struct EqlTraits {
    /// The type implements equality between its values using the `=` operator.
    pub eq: bool,

    /// The type implements comparison of its values using `>`, `>=`, `=`, `<=`, `<`.
    /// `ord` implies `eq`.
    pub ord: bool,

    /// The type implements textual substring search using `LIKE`.
    pub token_match: bool,

    /// The type implements field selection (e.g. `->` & `->>`)
    pub json_like: bool,

    /// The type implements containment checking (e.g. `@>` and `<@`)
    pub contain: bool,
}

pub const ALL_TRAITS: EqlTraits = EqlTraits {
    eq: true,
    ord: true,
    token_match: true,
    json_like: true,
    contain: true,
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
    pub fn none() -> Self {
        Self::default()
    }

    pub fn all() -> Self {
        ALL_TRAITS
    }

    pub(crate) fn add_mut(&mut self, eql_trait: EqlTrait) {
        match eql_trait {
            EqlTrait::Eq => self.eq = true,
            EqlTrait::Ord => {
                self.ord = true;
                self.eq = true; // implied by Ord
            }
            EqlTrait::TokenMatch => self.token_match = true,
            EqlTrait::JsonLike => {
                self.json_like = true;
            }
            EqlTrait::Contain => self.contain = true,
        }
    }

    pub(crate) fn union(&self, other: &Self) -> Self {
        EqlTraits {
            eq: self.eq || other.eq,
            ord: self.ord || other.ord,
            token_match: self.token_match || other.token_match,
            json_like: self.json_like || other.json_like,
            contain: self.contain || other.contain,
        }
    }

    pub(crate) fn intersection(&self, other: &Self) -> Self {
        EqlTraits {
            eq: self.eq && other.eq,
            ord: self.ord && other.ord,
            token_match: self.token_match && other.token_match,
            json_like: self.json_like && other.json_like,
            contain: self.contain && other.contain,
        }
    }

    pub(crate) fn difference(&self, other: &Self) -> Self {
        EqlTraits {
            eq: self.eq ^ other.eq,
            ord: self.ord ^ other.ord,
            token_match: self.token_match ^ other.token_match,
            json_like: self.json_like ^ other.json_like,
            contain: self.contain ^ other.contain,
        }
    }
}

impl std::fmt::Display for EqlTraits {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        const EQ: &str = "Eq";
        const ORD: &str = "Ord";
        const TOKEN_MATCH: &str = "TokenMatch";
        const CONTAIN: &str = "Contain";
        const JSON_LIKE: &str = "JsonLike";

        let mut traits: Vec<&'static str> = Vec::new();
        if self.eq {
            traits.push(EQ)
        }
        if self.ord {
            traits.push(ORD)
        }
        if self.token_match {
            traits.push(TOKEN_MATCH)
        }
        if self.contain {
            traits.push(CONTAIN)
        }
        if self.json_like {
            traits.push(JSON_LIKE)
        }

        f.write_str(&traits.join("+"))?;

        Ok(())
    }
}

impl Type {
    pub(crate) fn effective_bounds(&self) -> EqlTraits {
        match self {
            Type::Constructor(constructor) => constructor.effective_bounds(),
            Type::Var(Var(_, bounds)) => *bounds,
            Type::Associated(associated_type) => associated_type.resolved_ty.effective_bounds(),
        }
    }
}

impl Constructor {
    pub(crate) fn effective_bounds(&self) -> EqlTraits {
        match self {
            Constructor::Value(value) => value.effective_bounds(),
            Constructor::Projection(projection) => projection.effective_bounds(),
            Constructor::SetOf(ty) => ty.effective_bounds(),
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

impl SetOf {
    pub(crate) fn effective_bounds(&self) -> EqlTraits {
        let SetOf(some_ty) = self;
        some_ty.effective_bounds()
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
            EqlTerm::Partial(_, bounds) => *bounds,
            EqlTerm::JsonAccessor(_) => EqlTraits::none(),
            EqlTerm::JsonPath(_) => EqlTraits::none(),
            EqlTerm::Tokenized(_) => EqlTraits::none(),
        }
    }
}

impl EqlValue {
    pub(crate) fn effective_bounds(&self) -> EqlTraits {
        self.trait_impls()
    }
}

/*
    TODO: the following represents how I would eventually like to define the traits.

   /// `COUNT` has to be declared in order to work with EQL types.
   function pg_catalog.count<T>(T) -> Native;

   /// Trait that corresponds to equality tests in SQL.
   eqltrait Eq {
       /// The most minimal encoding of `Self` that can still be used by EQL (the database extension) to perform
       /// equality tests.  The purpose of `Partial` is to avoid generating all of the non-`Eq` search terms of
       /// `Self` if they are not going to be used.
       type Only;

       expr BinaryOp (Self = Self) -> Native;
       expr BinaryOp (Self <> Self) -> Native;
   }

   /// Trait that corresponds to comparison tests in SQL.
   eqltrait Ord: Eq {
       /// The most minimal encoding of `Self` that can still be used by EQL (the database extension) to perform
       /// comparison tests.  The purpose of `Only` is to avoid generating all of the non-`Ord` search terms of
       /// `Self` if they are not going to be used.
       type Only;

       binop (Self <= Self) -> Native;
       binop (Self >= Self) -> Native;
       binop (Self < Self) -> Native;
       binop (Self > Self) -> Native;

       fn pg_catalog.min(Self) -> Self;
       fn pg_catalog.max(Self) -> Self;
   }

   /// Trait that corresponds to containment testing operations in SQL.
   eqltrait Contain {
       type Only;

       binop (Self @> Self) -> Native;
       binop (Self <@ Self) -> Native;
   }

   /// Trait that corresponds to JSON/B operations in SQL.
   eqltrait JsonLike {
       /// A term that can select a field by name or an array element by index on `Self`.
       type Accessor;

       /// A term that can be used to match an entire JSON path on `Self`.
       type Path;

       binop (Self -> Self::Accessor) -> Self;
       binop (Self ->> Self::Accessor) -> Self;

       fn pg_catalog.jsonb_path_query(Self, Self::Path) -> Self;
       fn pg_catalog.jsonb_path_query_first(Self, Self::Path) -> Self;
       fn pg_catalog.jsonb_path_exists(Self, Self::Path) -> Native;
       fn pg_catalog.jsonb_array_length(Self) -> Native;
       fn pg_catalog.jsonb_array_elements(Self) -> {Self};
       fn pg_catalog.jsonb_array_elements_text(Self) -> {Self};
   }

   /// Trait that corresponds to LIKE operations in SQL.
   eqltrait TokenMatch {
       type Tokenized;

       binop (Self ~~ Self::Tokenized) -> Native;
       binop (Self !~~ Self::Tokenized) -> Native;

       LIKE { expr: Self, pattern: Self::Tokenized, .. } -> Native;
   }

   /// Trait that corresponds to LIKE & ILIKE operations in SQL.
   eqltrait TokenMatchCaseInsensitive: TokenMatch {
       binop (Self ~~* Self::Tokenized) -> Native;
       binop (Self !~~* Self::Tokenized) -> Native;

       ILIKE { expr: Self, pattern: Self::Tokenized, .. } -> Native;
   }

   /// The type used by the EQL Mapper to represent any non-EQL type.
   #[derive(Eq, Ord, Contain, JsonLike, TokenMatch)]
   #[lang-item]
   type Native;
*/
