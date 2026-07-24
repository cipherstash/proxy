use std::{any::type_name, ops::Index, sync::Arc};

use derive_more::Display;
use sqltk::parser::ast::Ident;

use crate::{ColumnKind, Table, TypeError};

use super::{EqlTrait, EqlTraits, Unifier};

/// The [`Type`] enum represents the types used by the [`Unifier`] to represent the SQL & EQL types returned by
/// expressions, projection-producing statements, built-in database functions & operators, EQL function & operators and
/// table columns.
///
/// A value of [`Type`] is either a [`Constructor`] (a fully or partially resolved type) or a [`TypeVar`] (a placeholder
/// for an unresolved type) or [`Associated`] (an associated type).
///
/// After successful unification of all of the types in a SQL statement, the types are converted into the publicly
/// exported [`crate::Type`] type, which is a mirror of this enum but without type variables which makes it more
/// ergonomic to consume.
#[derive(Debug, PartialEq, PartialOrd, Ord, Eq, Clone, Display, Hash)]
pub enum Type {
    /// A value type.
    #[display("{}", _0)]
    Value(Value),

    /// A type representing a placeholder for an unresolved type.
    #[display("{}", _0)]
    Var(Var),

    /// An associated type declared in an [`EqlTrait`] and implemented by a type that implements the `EqlTrait`.
    #[display("{}", _0)]
    Associated(AssociatedType),
}

impl Type {
    pub fn contains_eql(&self) -> bool {
        match self {
            Type::Value(value) => value.contains_eql(),
            Type::Var(_) => false,
            Type::Associated(associated_type) => associated_type.contains_eql(),
        }
    }
}

/// An associated type.
///
/// This is a type of the form `T::A`. `T` is the type that implements a trait that defines the associated type. `A` is
/// the associated type.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, derive_more::Display)]
#[display("<{} as {}>::{}", impl_ty, selector.eql_trait, selector.type_name)]
pub struct AssociatedType {
    /// A value that can resolve the concrete `A` when given a concrete `T`.
    pub selector: AssociatedTypeSelector,

    /// The type that implements the trait and will have defined an associated type. In `T::A` `impl_ty` is the `T`.
    pub impl_ty: Arc<Type>,

    /// The associated type itself. In `T::A` `resolved_ty` is the `A`.
    pub resolved_ty: Arc<Type>,
}

impl AssociatedType {
    /// Tries to resolve the concrete associated type.
    ///
    /// If the parent type that the associated type is attached to is not yet resolved then this method will return
    /// `Ok(None)`.
    pub(crate) fn resolve_selector_target(
        &self,
        unifier: &mut Unifier<'_>,
    ) -> Result<Option<Arc<Type>>, TypeError> {
        let impl_ty = self.impl_ty.clone().follow_tvars(unifier);
        if let Type::Value(_) = &*impl_ty {
            // The type that implements the EqlTrait is now known, so resolve the selector.
            let ty: Arc<Type> = self.selector.resolve(impl_ty.clone())?;
            Ok(Some(unifier.unify(self.resolved_ty.clone(), ty.clone())?))
        } else {
            Ok(None)
        }
    }

    fn contains_eql(&self) -> bool {
        self.impl_ty.contains_eql()
    }
}

/// A type variable with trait bounds.
///
/// Type variables represent an unresolved type. Unification of a concrete type with a type variable will succeed if the
/// concrete type implements all of the bounds on the type variable. The concrete type is allowed to implement a set of
/// traits that exceed the requirements of the bounds on the type variable.
#[derive(Debug, PartialEq, PartialOrd, Ord, Eq, Clone, Hash)]
pub struct Var(pub TypeVar, pub EqlTraits);

impl Display for Var {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.1 != EqlTraits::none() {
            f.write_fmt(format_args!("{}: {}", self.0, self.1))
        } else {
            f.write_fmt(format_args!("{}", self.0))
        }
    }
}

/// Represents a SQL `setof` type. Functions such as `jsonb_array_elements` return a `seto`.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Display, Hash)]
pub struct SetOf(pub Arc<Type>);

impl SetOf {
    pub(crate) fn inner_ty(&self) -> Arc<Type> {
        self.0.clone()
    }

    fn contains_eql(&self) -> bool {
        self.0.contains_eql()
    }
}

/// The type of SQL expression.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Display, Hash)]
pub enum Value {
    /// An encrypted type from a particular table-column in the schema.
    ///
    /// An encrypted column never shares a type with another encrypted column - which is why it is sufficient to
    /// identify the type by its table & column names.
    #[display("{}", _0)]
    Eql(EqlTerm),

    /// A native database type that carries its table & column name.  `NativeValue(None)` & `NativeValue(Some(_))` are
    /// will successfully unify with each other - they are the same type as far as the type system is concerned.
    /// `NativeValue(Some(_))` just carries more information which makes testing & debugging easier.
    #[display("{}", _0)]
    Native(NativeValue),

    /// An array type that is parameterized by an element type.
    #[display("Array[{}]", _0)]
    Array(Array),

    /// A projection is a type with a fixed number of columns each of which has a type and optional alias.
    #[display("{}", _0)]
    Projection(Projection),

    /// In PostgreSQL, SETOF is a special return type used in functions to indicate that the function returns a set of
    /// rows rather than a single value. It allows a function to behave like a table or subquery in SQL, producing
    /// multiple rows as output.
    #[display("{}", _0)]
    SetOf(SetOf),
}

impl Value {
    pub fn contains_eql(&self) -> bool {
        match self {
            Value::Eql(_) => true,
            Value::Native(_) => false,
            Value::Array(array) => array.contains_eql(),
            Value::Projection(projection) => projection.contains_eql(),
            Value::SetOf(set_of) => set_of.contains_eql(),
        }
    }
}

/// An array of some type.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Display, Hash)]
pub struct Array(pub Arc<Type>);

impl Array {
    fn contains_eql(&self) -> bool {
        self.0.contains_eql()
    }
}

/// An `EqlTerm` is a type associated with a particular EQL type, i.e. an [`EqlValue`].
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Display, Hash)]
pub enum EqlTerm {
    /// This type represents the entire EQL payload (ciphertext + all encrypted search terms).  It is suitable both for
    /// `INSERT`ing new records and for querying against.
    #[display("EQL:Full({})", _0)]
    Full(EqlValue),

    /// This type represents a an EQL payload with exactly the encrypted search terms required in order to satisy its
    /// [`Bounds`].
    #[display("EQL:Partial({}: {})", _0, _1)]
    Partial(EqlValue, EqlTraits),

    /// A JSON field or array index. The inferred type of the right hand side of the `->` operator when the
    /// left hand side is an [`EqlValue`] that implements the EQL trait `JsonLike`.
    #[display("EQL:JsonAccessor({})", _0)]
    JsonAccessor(EqlValue),

    /// A JSON path. The inferred type of the second argument to functions such `jsonb_path_query` when the first
    /// argument is an [`EqlValue`] that implements the EQL trait `JsonLike`.
    #[display("EQL:JsonPath({})", _0)]
    JsonPath(EqlValue),

    /// A text value that can be used as the right hand side of `LIKE` or `ILIKE` when the left hand side is an
    /// [`EqlValue`] that implements the EQL trait `TokenMatch`.
    #[display("EQL:Tokenized({})", _0)]
    Tokenized(EqlValue),
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Copy, Display, Hash)]
pub enum EqlTermVariant {
    #[display("EQL:Full")]
    Full,
    #[display("EQL:Partial")]
    Partial,
    #[display("EQL:JsonAccessor")]
    JsonAccessor,
    #[display("EQL:JsonPath")]
    JsonPath,
    #[display("EQL:Tokenized")]
    Tokenized,
}

impl EqlTerm {
    pub fn table_column(&self) -> &TableColumn {
        self.eql_value().table_column()
    }

    /// The [`EqlValue`] every `EqlTerm` variant wraps — its `TableColumn`, inert
    /// domain identity, and capabilities.
    pub fn eql_value(&self) -> &EqlValue {
        match self {
            EqlTerm::Full(eql_value)
            | EqlTerm::Partial(eql_value, _)
            | EqlTerm::JsonAccessor(eql_value)
            | EqlTerm::JsonPath(eql_value)
            | EqlTerm::Tokenized(eql_value) => eql_value,
        }
    }

    pub fn variant(&self) -> EqlTermVariant {
        match self {
            EqlTerm::Full(_) => EqlTermVariant::Full,
            EqlTerm::Partial(_, _) => EqlTermVariant::Partial,
            EqlTerm::JsonAccessor(_) => EqlTermVariant::JsonAccessor,
            EqlTerm::JsonPath(_) => EqlTermVariant::JsonPath,
            EqlTerm::Tokenized(_) => EqlTermVariant::Tokenized,
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, derive_more::Display)]
#[display("{eql_trait}::{type_name}")]
pub struct AssociatedTypeSelector {
    pub eql_trait: EqlTrait,
    pub type_name: &'static str,
}

impl AssociatedTypeSelector {
    pub(crate) fn new(
        eql_trait: EqlTrait,
        associated_type_name: &'static str,
    ) -> Result<Self, TypeError> {
        if eql_trait.has_associated_type(associated_type_name) {
            Ok(Self {
                eql_trait,
                type_name: associated_type_name,
            })
        } else {
            Err(TypeError::InternalError(format!(
                "Trait {eql_trait} does not define associated type {associated_type_name}"
            )))
        }
    }

    pub(crate) fn resolve(&self, ty: Arc<Type>) -> Result<Arc<Type>, TypeError> {
        Ok(self.eql_trait.resolve_associated_type(ty, self)?.clone())
    }
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Display, Hash)]
#[display("{}.{}", table, column)]
pub struct TableColumn {
    pub table: Ident,
    pub column: Ident,
}

/// The plaintext scalar half of a v3 domain — the "token type".
///
/// Crossed with a capability suffix (`_eq`, `_ord`, …) it names a v3 domain,
/// e.g. `text` + `_ord_ore` ⇒ `eql_v3_text_ord_ore`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Display)]
pub enum TokenType {
    SmallInt,
    Integer,
    BigInt,
    Real,
    Double,
    Numeric,
    Text,
    Boolean,
    Date,
    Timestamp,
    Json,
}

impl TokenType {
    /// The token type's spelling inside a v3 domain typname
    /// (`eql_v3_<token>_<suffix>`).
    pub fn as_domain_str(&self) -> &'static str {
        match self {
            TokenType::SmallInt => "smallint",
            TokenType::Integer => "integer",
            TokenType::BigInt => "bigint",
            TokenType::Real => "real",
            TokenType::Double => "double",
            TokenType::Numeric => "numeric",
            TokenType::Text => "text",
            TokenType::Boolean => "boolean",
            TokenType::Date => "date",
            TokenType::Timestamp => "timestamp",
            TokenType::Json => "json",
        }
    }

    /// Parse the token type from a v3 domain typname. The token type is the
    /// first segment after the `eql_v3_` prefix; every token type is a single
    /// underscore-free word, so a multi-part capability suffix never interferes.
    pub fn from_domain_name(domain: &str) -> Option<Self> {
        let rest = domain.strip_prefix("eql_v3_")?;
        Some(match rest.split('_').next()? {
            "smallint" => TokenType::SmallInt,
            "integer" => TokenType::Integer,
            "bigint" => TokenType::BigInt,
            "real" => TokenType::Real,
            "double" => TokenType::Double,
            "numeric" => TokenType::Numeric,
            "text" => TokenType::Text,
            "boolean" => TokenType::Boolean,
            "date" => TokenType::Date,
            "timestamp" => TokenType::Timestamp,
            "json" => TokenType::Json,
            _ => return None,
        })
    }
}

/// The inert `(token type, v3 domain)` an encrypted column carries (ADR-0002).
///
/// Populated by the schema loader from the Postgres domain name; **never** a
/// checked dimension of unification. It is read only at rewrite time — to name
/// the cast target and to select the term-extraction-function variant
/// (`ord_term` vs `ord_term_ore`) — so it threads through unification and the
/// associated-type machinery untouched.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Display)]
#[display("{}", domain)]
pub struct DomainIdentity {
    pub token: TokenType,
    /// The v3 domain typname, e.g. `eql_v3_text_ord_ore`.
    pub domain: Ident,
}

impl DomainIdentity {
    /// Build an identity from a v3 domain typname, parsing the token type from
    /// the name. The domain name is the authority (the schema loader passes the
    /// real typname); returns `None` for a name that is not a v3 EQL domain.
    pub fn from_domain_name(domain: &str) -> Option<Self> {
        Some(Self {
            token: TokenType::from_domain_name(domain)?,
            domain: Ident::new(domain),
        })
    }

    /// The capability suffix of the domain typname (`eql_v3_<token>_<suffix>`),
    /// e.g. `ord_ore` for `eql_v3_text_ord_ore`, or `""` for a storage-only
    /// domain like `eql_v3_integer`.
    fn suffix(&self) -> &str {
        let prefix_len = "eql_v3_".len() + self.token.as_domain_str().len();
        self.domain
            .value
            .get(prefix_len..)
            .map(|rest| rest.strip_prefix('_').unwrap_or(rest))
            .unwrap_or("")
    }

    // Which SEM terms the domain stores, derived from its typname. The catalog is
    // the authority (ADR-0002) and these mirror the term → domain mapping the
    // schema loader inverts. `text` is the exception: `text_ord*` stores `hm`
    // alongside its ordering term, because lexicographic ORE/OPE over text is not
    // equality-lossless.

    /// The domain stores the `hm` (HMAC equality) term ⇒ `eq_term` is available.
    pub fn stores_hm(&self) -> bool {
        matches!(self.suffix(), "eq" | "search" | "search_ore")
            || (self.token == TokenType::Text
                && matches!(self.suffix(), "ord" | "ord_ope" | "ord_ore"))
    }

    /// The domain stores the `op` (CLLW-OPE) term ⇒ `ord_term` is available.
    pub fn stores_op(&self) -> bool {
        matches!(self.suffix(), "ord" | "ord_ope" | "search")
    }

    /// The domain stores the `ob` (block-ORE) term ⇒ `ord_term_ore` is available.
    pub fn stores_ob(&self) -> bool {
        matches!(self.suffix(), "ord_ore" | "search_ore")
    }

    /// The domain stores the `bf` (bloom-filter) term ⇒ `match_term` is available.
    pub fn stores_bf(&self) -> bool {
        matches!(self.suffix(), "match" | "search" | "search_ore")
    }

    /// The `eql_v3` term-extraction function for equality (`=`, `<>`), or `None`
    /// if the domain supports no equality. `eq_term` when the domain stores `hm`;
    /// otherwise equality falls back to the ordering term (an ord-only scalar such
    /// as `integer_ord` compares via `ord_term`, mirroring `eql_v3.eq`).
    pub fn eq_term_fn(&self) -> Option<&'static str> {
        if self.stores_hm() {
            Some("eq_term")
        } else {
            self.ord_term_fn()
        }
    }

    /// The `eql_v3` term-extraction function for ordering (`<`, `<=`, `>`, `>=`,
    /// `MIN`/`MAX`), or `None` if the domain is not orderable. `ord_term` for `op`
    /// domains, `ord_term_ore` for `ob` (block-ORE) domains.
    pub fn ord_term_fn(&self) -> Option<&'static str> {
        if self.stores_op() {
            Some("ord_term")
        } else if self.stores_ob() {
            Some("ord_term_ore")
        } else {
            None
        }
    }

    /// The `eql_v3` term-extraction function for fuzzy match (`@@`), or `None` if
    /// the domain has no bloom filter.
    pub fn match_term_fn(&self) -> Option<&'static str> {
        if self.stores_bf() {
            Some("match_term")
        } else {
            None
        }
    }

    /// The query-operand twin of this column domain — `(schema, typname)`, e.g.
    /// `("eql_v3", "query_integer_ord")` for `public.eql_v3_integer_ord`. A query
    /// operand casts to the twin (which carries the term-only payload), never to
    /// the column domain (whose CHECK requires the stored ciphertext).
    pub fn query_twin(&self) -> (&'static str, String) {
        // Every JSON domain (`json`, `json_search`, `json_entry`) shares a single
        // query-operand type in the catalog — `eql_v3.query_json` — because a
        // jsonb query operand is a SteVec needle whose shape does not vary by the
        // column's searchable capability. The generic `query_<bare>` rule below is
        // correct only for the scalar families (e.g. `query_integer_ord`); applied
        // to JSON it would emit a non-existent `eql_v3.query_json_search`.
        if self.token == TokenType::Json {
            return ("eql_v3", "query_json".to_string());
        }
        let bare = self
            .domain
            .value
            .strip_prefix("eql_v3_")
            .unwrap_or(&self.domain.value);
        ("eql_v3", format!("query_{bare}"))
    }

    /// A canonical identity for a `(token, capabilities)` pair. This is a
    /// **test/fixture convenience** for constructing identities where no live
    /// schema loader supplies the real domain name — production identities always
    /// come from [`Self::from_domain_name`] via the loader. The synthesised domain
    /// name is deterministic so both sides of a test assertion agree.
    ///
    /// The synthesised name is NOT authoritative and must never be treated as the
    /// column's real catalog domain: it may not only diverge from the real typname
    /// but actively **collide with an unrelated real domain that means something
    /// else**. For example `canonical(Text, {json_like})` produces
    /// `eql_v3_text_search` — a real catalog domain whose terms are `[hm, op, bf]`
    /// (Eq + Ord + TokenMatch), nothing to do with JSON — and it can equally emit
    /// genuinely non-catalog names (`Eq + TokenMatch` → `eql_v3_text_eq_match`,
    /// `Contain` → `eql_v3_text_contain`). Only [`Self::from_domain_name`] yields a
    /// real domain identity.
    pub fn canonical(token: TokenType, traits: EqlTraits) -> Self {
        let mut parts: Vec<&str> = Vec::new();
        if traits.json_like {
            parts.push("search");
        } else {
            if traits.ord {
                parts.push("ord");
            } else if traits.eq {
                parts.push("eq");
            }
            if traits.token_match {
                parts.push("match");
            }
            if traits.contain {
                parts.push("contain");
            }
        }
        let suffix = if parts.is_empty() {
            String::new()
        } else {
            format!("_{}", parts.join("_"))
        };
        let domain = format!("eql_v3_{}{}", token.as_domain_str(), suffix);
        Self {
            token,
            domain: Ident::new(domain),
        }
    }
}

/// The identity of an encrypted column: its `TableColumn`, its inert
/// [`DomainIdentity`] (see ADR-0002), and its [`EqlTraits`] capabilities.
///
/// The domain identity is deliberately not part of `PartialEq`/`Ord`-driven
/// unification — two encrypted columns never share a type because their
/// `TableColumn`s differ, so the identity never decides unification even though
/// it is compared here.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Display, Hash)]
#[display("EQL({})", _0)]
pub struct EqlValue(pub TableColumn, pub DomainIdentity, pub EqlTraits);

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Display, Hash)]
#[display("{}", _0.as_ref().map(|tc| format!("Native({tc})")).unwrap_or(String::from("Native")))]
pub struct NativeValue(pub Option<TableColumn>);

/// A column from a projection.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Display, Hash)]
#[display("{}{}", self.ty, self.render_alias())]
pub struct ProjectionColumn {
    /// The type of the column.
    pub ty: Arc<Type>,

    /// The columm alias
    pub alias: Option<Ident>,
}

/// A placeholder for an unknown type.
#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Display)]
#[display("?{}", _0)]
pub struct TypeVar(pub usize);

impl From<TypeVar> for Type {
    fn from(tvar: TypeVar) -> Self {
        Type::Var(Var(tvar, EqlTraits::none()))
    }
}

impl Type {
    /// Creates a `Type::Value(Projection::Empty)`.
    pub(crate) const fn empty_projection() -> Type {
        Type::Value(Value::Projection(Projection(vec![])))
    }

    /// Creates a `Type::Value(Value::Native(NativeValue(None)))`.
    pub(crate) const fn native() -> Type {
        Type::Value(Value::Native(NativeValue(None)))
    }

    /// Creates a `Type::Value(Value::SetOf(ty))`.
    pub(crate) const fn set_of(ty: Arc<Type>) -> Type {
        Type::Value(Value::SetOf(SetOf(ty)))
    }

    /// Creates a `Type::Value(Value::Projection(Projection::WithColumns(columns)))`.
    pub(crate) fn projection(columns: &[(Arc<Type>, Option<Ident>)]) -> Type {
        Type::Value(Value::Projection(Projection(
            columns
                .iter()
                .map(|(c, n)| ProjectionColumn::new(c.clone(), n.clone()))
                .collect(),
        )))
    }

    /// Creates a `Type::Value(Value::Array(element_ty))`.
    pub(crate) fn array(element_ty: impl Into<Arc<Type>>) -> Arc<Type> {
        Type::Value(Value::Array(Array(element_ty.into()))).into()
    }

    /// Dereferences all type variables in `self` to the final type in chain of `Type::Var`.
    /// The final type can be a `Type::Var`.
    pub(crate) fn follow_tvars(self: Arc<Self>, unifier: &Unifier<'_>) -> Arc<Type> {
        match &*self.clone() {
            Type::Value(Value::Projection(Projection(cols))) => {
                let cols = cols
                    .iter()
                    .map(|col| ProjectionColumn {
                        ty: col.ty.clone().follow_tvars(unifier),
                        alias: col.alias.clone(),
                    })
                    .collect();
                Projection(cols).into()
            }

            Type::Value(Value::Array(Array(ty))) => Arc::new(Type::Value(Value::Array(Array(
                ty.clone().follow_tvars(unifier),
            )))),

            Type::Value(Value::SetOf(SetOf(ty))) => ty.clone().follow_tvars(unifier),

            Type::Value(_) => self,

            Type::Var(Var(tvar, _)) => {
                if let Some(ty) = unifier.get_type(*tvar) {
                    ty.follow_tvars(unifier)
                } else {
                    self
                }
            }

            Type::Associated(AssociatedType {
                impl_ty,
                resolved_ty,
                selector,
            }) => {
                let impl_ty = impl_ty.clone().follow_tvars(unifier);
                let resolved_ty = resolved_ty.clone().follow_tvars(unifier);

                Type::Associated(AssociatedType {
                    impl_ty,
                    resolved_ty,
                    selector: selector.clone(),
                })
                .into()
            }
        }
    }

    pub(crate) fn resolved_as<T: Clone + 'static>(
        self: Arc<Type>,
        unifier: &Unifier<'_>,
    ) -> Result<T, TypeError> {
        let resolved_ty = self.follow_tvars(unifier);

        if !matches!(&*resolved_ty, Type::Value(_)) {
            return Err(TypeError::Expected("type to be resolved".to_string()));
        }

        let result = match &*resolved_ty {
            Type::Value(Value::Projection(projection)) => {
                if let Some(t) = (projection as &dyn std::any::Any).downcast_ref::<T>() {
                    return Ok(t.clone());
                }

                Err(())
            }
            Type::Value(Value::SetOf(ty)) => {
                if let Some(t) = (ty as &dyn std::any::Any).downcast_ref::<T>() {
                    return Ok(t.clone());
                }

                Err(())
            }
            Type::Value(value) => {
                match value {
                    Value::Eql(maybe_t) => {
                        if let Some(t) = (maybe_t as &dyn std::any::Any).downcast_ref::<T>() {
                            return Ok(t.clone());
                        }
                    }
                    Value::Native(maybe_t) => {
                        if let Some(t) = (maybe_t as &dyn std::any::Any).downcast_ref::<T>() {
                            return Ok(t.clone());
                        }
                    }
                    Value::Array(maybe_t) => {
                        if let Some(t) = (maybe_t as &dyn std::any::Any).downcast_ref::<T>() {
                            return Ok(t.clone());
                        }
                    }
                    Value::Projection(maybe_t) => {
                        if let Some(t) = (maybe_t as &dyn std::any::Any).downcast_ref::<T>() {
                            return Ok(t.clone());
                        }
                    }
                    Value::SetOf(maybe_t) => {
                        if let Some(t) = (maybe_t as &dyn std::any::Any).downcast_ref::<T>() {
                            return Ok(t.clone());
                        }
                    }
                }

                Err(())
            }
            Type::Associated(_) => Err(()),
            Type::Var(_) => Err(()),
        };

        result.map_err(|_| {
            TypeError::InternalError(format!(
                "could not resolve type {} as {}",
                resolved_ty,
                type_name::<T>()
            ))
        })
    }

    pub(crate) fn must_implement(&self, bounds: &EqlTraits) -> Result<(), TypeError> {
        if self.effective_bounds().intersection(bounds) == *bounds {
            Ok(())
        } else {
            Err(TypeError::UnsatisfiedBounds(
                Arc::new(self.clone()),
                // Report the *missing* bounds: required (`bounds`) minus implemented
                // (`self.effective_bounds()`). Operand order must match
                // `Unifier::satisfy_bounds`.
                bounds.difference(&self.effective_bounds()),
            ))
        }
    }
}

impl EqlValue {
    pub fn table_column(&self) -> &TableColumn {
        &self.0
    }

    /// The inert v3 domain identity — names the cast target and selects the
    /// term-extraction-function variant at rewrite time (ADR-0002).
    pub fn domain_identity(&self) -> &DomainIdentity {
        &self.1
    }

    /// Test/fixture constructor: builds the value with the canonical `text`-token
    /// [`DomainIdentity`] for `traits`. Production values come from the schema
    /// loader with the real domain identity, never this.
    pub fn with_canonical_identity(table_column: TableColumn, traits: EqlTraits) -> Self {
        Self(
            table_column,
            DomainIdentity::canonical(TokenType::Text, traits),
            traits,
        )
    }

    pub fn trait_impls(&self) -> EqlTraits {
        self.2
    }
}

/// The type of an [`sqltk::parser::ast::Expr`] or [`sqltk::parser::ast::Statement`] that returns a projection.
///
/// It represents an ordered list of zero or more optionally aliased columns types.
///
/// An `INSERT`, `UPDATE` or `DELETE` statement without a `RETURNING` clause will have an empty projection.
///
/// Also statements such as `SELECT FROM users` where there are no selected columns or wildcards will have an empty
/// projection.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Hash)]
pub struct Projection(pub Vec<ProjectionColumn>);

impl Display for Projection {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("{")?;
        for (idx, col) in self.0.iter().enumerate() {
            col.fmt(f)?;
            if idx < self.0.len() - 1 {
                f.write_str(", ")?;
            }
        }
        f.write_str("}")
    }
}

impl Projection {
    pub fn new(columns: Vec<ProjectionColumn>) -> Self {
        Self(columns)
    }

    pub(crate) fn new_from_schema_table(table: Arc<Table>) -> Self {
        Self(
            table
                .columns
                .iter()
                .map(|col| {
                    let tc = TableColumn {
                        table: table.name.clone(),
                        column: col.name.clone(),
                    };

                    let value_ty = match &col.kind {
                        ColumnKind::Native => Type::Value(Value::Native(NativeValue(Some(tc)))),
                        ColumnKind::Eql(features, identity) => Type::Value(Value::Eql(
                            EqlTerm::Full(EqlValue(tc, identity.clone(), *features)),
                        )),
                    };

                    ProjectionColumn::new(value_ty, Some(col.name.clone()))
                })
                .collect(),
        )
    }

    pub fn len(&self) -> usize {
        self.0.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn columns(&self) -> &[ProjectionColumn] {
        &self.0
    }

    fn contains_eql(&self) -> bool {
        self.columns().iter().any(|col| col.ty.contains_eql())
    }

    pub(crate) fn flatten(&self, unifier: &Unifier<'_>) -> Result<Self, TypeError> {
        let resolved_cols = self.columns().iter().try_fold(
            vec![],
            |mut acc, col| -> Result<Vec<ProjectionColumn>, TypeError> {
                let alias = col.alias.clone();
                if let Type::Value(Value::Projection(projection)) =
                    &*col.ty.clone().follow_tvars(unifier)
                {
                    let resolved = projection.flatten(unifier)?;
                    acc.extend(resolved.0);
                } else {
                    let ty = col.ty.clone().follow_tvars(unifier);
                    acc.push(ProjectionColumn { ty, alias });
                }
                Ok(acc)
            },
        )?;

        Ok(crate::Projection(resolved_cols))
    }
}

impl Index<usize> for Projection {
    type Output = ProjectionColumn;

    fn index(&self, index: usize) -> &Self::Output {
        &self.0[index]
    }
}

impl ProjectionColumn {
    /// Returns a new `ProjectionColumn` with type `ty` and optional `alias`.
    pub(crate) fn new(ty: impl Into<Arc<Type>>, alias: Option<Ident>) -> Self {
        let ty: Arc<Type> = ty.into();
        Self {
            ty: ty.clone(),
            alias,
        }
    }

    fn render_alias(&self) -> String {
        match &self.alias {
            Some(name) => format!(": {name}"),
            None => String::from(""),
        }
    }
}

macro_rules! impl_from_for_arc_type {
    ($ty:ty) => {
        impl From<$ty> for Arc<Type> {
            fn from(value: $ty) -> Self {
                Arc::new(Type::from(value))
            }
        }
    };
}

impl_from_for_arc_type!(NativeValue);
impl_from_for_arc_type!(Projection);
impl_from_for_arc_type!(Var);
impl_from_for_arc_type!(EqlTerm);
impl_from_for_arc_type!(Value);
impl_from_for_arc_type!(Array);
impl_from_for_arc_type!(AssociatedType);

impl From<AssociatedType> for Type {
    fn from(associated: AssociatedType) -> Self {
        Type::Associated(associated)
    }
}

impl From<Value> for Type {
    fn from(value: Value) -> Self {
        Type::Value(value)
    }
}

impl From<EqlTerm> for Type {
    fn from(eql_term: EqlTerm) -> Self {
        Type::Value(Value::Eql(eql_term))
    }
}

impl From<Var> for Type {
    fn from(var: Var) -> Self {
        Type::Var(var)
    }
}

impl From<Projection> for Type {
    fn from(projection: Projection) -> Self {
        Type::Value(Value::Projection(projection))
    }
}

impl From<NativeValue> for Type {
    fn from(native: NativeValue) -> Self {
        Type::Value(Value::Native(native))
    }
}

impl From<Array> for Type {
    fn from(array: Array) -> Self {
        Type::Value(Value::Array(array))
    }
}

#[cfg(test)]
mod domain_identity_tests {
    use super::DomainIdentity;

    fn di(domain: &str) -> DomainIdentity {
        DomainIdentity::from_domain_name(domain)
            .unwrap_or_else(|| panic!("{domain} is not a v3 domain name"))
    }

    #[test]
    fn suffix_is_parsed_across_tokens_and_variants() {
        assert_eq!(di("eql_v3_integer").suffix(), "");
        assert_eq!(di("eql_v3_integer_eq").suffix(), "eq");
        assert_eq!(di("eql_v3_integer_ord").suffix(), "ord");
        assert_eq!(di("eql_v3_integer_ord_ope").suffix(), "ord_ope");
        assert_eq!(di("eql_v3_integer_ord_ore").suffix(), "ord_ore");
        assert_eq!(di("eql_v3_text_search_ore").suffix(), "search_ore");
        assert_eq!(di("eql_v3_bigint_ord_ore").suffix(), "ord_ore");
    }

    #[test]
    fn double_token_does_not_swallow_the_capability_suffix() {
        // `double` is the one token whose plain-English name could be mistaken
        // for a two-word `double precision`. The catalog spells the domain
        // `eql_v3_double_ord` (see tests/sql/schema.sql), so `as_domain_str()`
        // ("double", 6 chars) must line the prefix up exactly on the `_ord`
        // boundary. Pins the invariant that `suffix()`/`stores_*` documents as a
        // comment: a hypothetical `eql_v3_double_precision_ord` would parse the
        // suffix as "precision_ord" and silently report the column as
        // non-orderable.
        assert_eq!(di("eql_v3_double").suffix(), "");
        assert_eq!(di("eql_v3_double_ord").suffix(), "ord");
        assert_eq!(di("eql_v3_double_ord_ore").suffix(), "ord_ore");
        assert_eq!(di("eql_v3_double_ord").ord_term_fn(), Some("ord_term"));
        assert_eq!(
            di("eql_v3_double_ord_ore").ord_term_fn(),
            Some("ord_term_ore")
        );
    }

    #[test]
    fn eq_term_uses_eq_term_only_when_hm_is_stored() {
        // _eq stores hm.
        assert_eq!(di("eql_v3_integer_eq").eq_term_fn(), Some("eq_term"));
        // ord-only scalar has no hm -> equality falls back to ord_term
        // (mirrors eql_v3.eq(integer_ord, ...) = ord_term(a) = ord_term(b)).
        assert_eq!(di("eql_v3_integer_ord").eq_term_fn(), Some("ord_term"));
        assert_eq!(
            di("eql_v3_integer_ord_ore").eq_term_fn(),
            Some("ord_term_ore")
        );
        // text is the exception: text_ord* stores hm, so eq_term is available.
        assert_eq!(di("eql_v3_text_ord").eq_term_fn(), Some("eq_term"));
        assert_eq!(di("eql_v3_text_ord_ore").eq_term_fn(), Some("eq_term"));
        // storage-only and match-only have no equality.
        assert_eq!(di("eql_v3_integer").eq_term_fn(), None);
        assert_eq!(di("eql_v3_text_match").eq_term_fn(), None);
    }

    #[test]
    fn ord_term_picks_ope_vs_ore_from_the_domain() {
        assert_eq!(di("eql_v3_integer_ord").ord_term_fn(), Some("ord_term"));
        assert_eq!(di("eql_v3_integer_ord_ope").ord_term_fn(), Some("ord_term"));
        assert_eq!(
            di("eql_v3_integer_ord_ore").ord_term_fn(),
            Some("ord_term_ore")
        );
        assert_eq!(di("eql_v3_text_search").ord_term_fn(), Some("ord_term"));
        assert_eq!(
            di("eql_v3_text_search_ore").ord_term_fn(),
            Some("ord_term_ore")
        );
        // not orderable
        assert_eq!(di("eql_v3_integer_eq").ord_term_fn(), None);
        assert_eq!(di("eql_v3_text_match").ord_term_fn(), None);
        assert_eq!(di("eql_v3_integer").ord_term_fn(), None);
    }

    #[test]
    fn match_term_needs_a_bloom_filter() {
        assert_eq!(di("eql_v3_text_match").match_term_fn(), Some("match_term"));
        assert_eq!(di("eql_v3_text_search").match_term_fn(), Some("match_term"));
        assert_eq!(
            di("eql_v3_text_search_ore").match_term_fn(),
            Some("match_term")
        );
        assert_eq!(di("eql_v3_text_ord").match_term_fn(), None);
        assert_eq!(di("eql_v3_integer_eq").match_term_fn(), None);
    }

    #[test]
    fn storage_only_domain_supports_no_operations() {
        let d = di("eql_v3_integer");
        assert_eq!(d.eq_term_fn(), None);
        assert_eq!(d.ord_term_fn(), None);
        assert_eq!(d.match_term_fn(), None);
    }

    #[test]
    fn query_twin_prefixes_the_bare_domain() {
        assert_eq!(
            di("eql_v3_integer_ord").query_twin(),
            ("eql_v3", "query_integer_ord".to_string())
        );
        assert_eq!(
            di("eql_v3_text_search_ore").query_twin(),
            ("eql_v3", "query_text_search_ore".to_string())
        );
    }

    #[test]
    fn json_domains_all_share_the_query_json_twin() {
        // The catalog defines a single jsonb query operand type, eql_v3.query_json,
        // for every JSON column domain — the generic query_<bare> rule would emit a
        // non-existent eql_v3.query_json_search / eql_v3.query_json_entry.
        assert_eq!(
            di("eql_v3_json").query_twin(),
            ("eql_v3", "query_json".to_string())
        );
        assert_eq!(
            di("eql_v3_json_search").query_twin(),
            ("eql_v3", "query_json".to_string())
        );
    }
}
