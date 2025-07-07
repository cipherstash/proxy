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
        match self {
            EqlTerm::Full(eql_value)
            | EqlTerm::Partial(eql_value, _)
            | EqlTerm::JsonAccessor(eql_value)
            | EqlTerm::JsonPath(eql_value)
            | EqlTerm::Tokenized(eql_value) => eql_value.table_column(),
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

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Display, Hash)]
#[display("EQL({})", _0)]
pub struct EqlValue(pub TableColumn, pub EqlTraits);

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Display, Hash)]
#[display("{}", _0.as_ref().map(|tc| format!("({tc})")).unwrap_or(String::from("")))]
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
                self.effective_bounds().difference(bounds),
            ))
        }
    }
}

impl EqlValue {
    pub fn table_column(&self) -> &TableColumn {
        &self.0
    }

    pub fn trait_impls(&self) -> EqlTraits {
        self.1
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
                        ColumnKind::Eql(features) => {
                            Type::Value(Value::Eql(EqlTerm::Full(EqlValue(tc, *features))))
                        }
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
                    acc.extend(resolved.0.into_iter());
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

// Statically assert that `Type` is `Send + Sync`.  If `Type` did not implement `Send` and/or `Sync` this crate would
// fail to compile anyway but the error message is very obtuse. A failure here makes it obvious.
const _: () = {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    fn assert_all() {
        assert_send::<Type>();
        assert_sync::<Type>();
    }
};
