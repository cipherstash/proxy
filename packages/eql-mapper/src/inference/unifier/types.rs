use std::{any::type_name, ops::Index, sync::Arc};

use derive_more::Display;
use sqltk::parser::ast::Ident;

use crate::{ColumnKind, Table, TypeError};

use super::{resolve_type::ResolveType, EqlTrait, EqlTraits, Unifier};

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
#[display("{self}")]
pub enum Type {
    /// A specific type constructor with zero or more generic parameters.
    #[display("{}", _0)]
    Constructor(Constructor),

    /// A type representing a placeholder for an unresolved type.
    #[display("{}", _0)]
    Var(Var),

    /// An associated type declared in an [`EqlTrait`] and implemented by a type that implements the `EqlTrait`.
    #[display("{}", _0)]
    Associated(AssociatedType),
}

/// An associated type.
///
/// This is a type of the form `T::A` - `T` is a parent type and `A` is an associated type (just like in Rust).
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, derive_more::Display)]
#[display("<{} as {}>::{}", impl_ty, selector.eql_trait, selector.type_name)]
pub struct AssociatedType {
    pub selector: AssociatedTypeSelector,

    /// The type that implements the trait and will have defined an associated type.
    pub impl_ty: Arc<Type>,

    /// An initially dangling type variable that will eventually unify with the resolved type.
    pub resolved_ty: Arc<Type>,
}

impl AssociatedType {
    pub(crate) fn resolve_selector_target(
        &self,
        unifier: &mut Unifier<'_>,
    ) -> Result<Option<Arc<Type>>, TypeError> {
        let impl_ty = self.impl_ty.clone().follow_tvars(unifier);
        if let Type::Constructor(_) = &*impl_ty {
            // The type that implements the EqlTrait is now known, so resolve the selector.
            let ty: Arc<Type> = self.selector.resolve(impl_ty.clone())?;
            Ok(Some(unifier.unify(self.resolved_ty.clone(), ty.clone())?))
        } else {
            Ok(None)
        }
    }
}

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

const _: () = {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    fn assert_all() {
        assert_send::<Type>();
        assert_sync::<Type>();
    }
};

/// A `Constructor` is what is known about a [`Type`].
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Display, Hash)]
pub enum Constructor {
    /// An EQL type, an opaque "database native" type or an array type.
    #[display("{}", _0)]
    Value(Value),

    /// A projection is a type with a fixed number of columns each of which has a type and optional alias.
    #[display("{}", _0)]
    Projection(Projection),
}

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
}

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Display, Hash)]
pub struct Array(pub Arc<Type>);

/// An `EqlTerm` is a type associated with a particular EQL type, i.e. an [`EqlValue`].
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Display, Hash)]
pub enum EqlTerm {
    /// This type represents the entire EQL payload (ciphertext + all encrypted search terms).  It is suitable both for
    /// `INSERT`ing new records and for querying against.
    #[display("EQL:Full({})", _0)]
    Full(EqlValue),

    /// This type represents a an EQL payload with exactly the encrypted search terms required in order to satisy its
    /// [`Bounds`].
    ///
    /// A `Partial` type can become a `Whole` type during unification.
    #[display("EQL:Partial({}: {})", _0, _1)]
    Partial(EqlValue, EqlTraits),

    JsonAccessor(EqlValue),

    JsonPath(EqlValue),

    Tokenized(EqlValue),
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
        Ok(self
            .eql_trait
            .resolve_associated_type(ty, self)?
            .clone())
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
#[display("NATIVE{}", _0.as_ref().map(|tc| format!("({})", tc)).unwrap_or(String::from("")))]
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
    /// Creates a `Type` containing an empty projection
    pub(crate) const fn empty_projection() -> Type {
        Type::Constructor(Constructor::Projection(Projection::Empty))
    }

    /// Creates a `Type` containing a `Constructor::Scalar(Scalar::Native(NativeValue(None)))`.
    pub(crate) const fn native() -> Type {
        Type::Constructor(Constructor::Value(Value::Native(NativeValue(None))))
    }

    /// Creates a `Type` containing a `Constructor::Projection`.
    pub(crate) fn projection(columns: &[(Arc<Type>, Option<Ident>)]) -> Type {
        if columns.is_empty() {
            Type::Constructor(Constructor::Projection(Projection::Empty))
        } else {
            Type::Constructor(Constructor::Projection(Projection::WithColumns(
                ProjectionColumns(
                    columns
                        .iter()
                        .map(|(c, n)| ProjectionColumn::new(c.clone(), n.clone()))
                        .collect(),
                ),
            )))
        }
    }

    /// Creates a `Type` containing a `Constructor::Array`.
    pub(crate) fn array(element_ty: impl Into<Arc<Type>>) -> Arc<Type> {
        Type::Constructor(Constructor::Value(Value::Array(Array(element_ty.into())))).into()
    }

    pub(crate) fn follow_tvars(self: Arc<Self>, unifier: &Unifier<'_>) -> Arc<Type> {
        match &*self.clone() {
            Type::Constructor(Constructor::Projection(Projection::WithColumns(
                ProjectionColumns(cols),
            ))) => {
                let cols = cols
                    .iter()
                    .map(|col| ProjectionColumn {
                        ty: col.ty.clone().follow_tvars(unifier),
                        alias: col.alias.clone(),
                    })
                    .collect();
                Projection::WithColumns(ProjectionColumns(cols)).into()
            }

            Type::Constructor(Constructor::Projection(Projection::Empty)) => self,

            Type::Constructor(Constructor::Value(Value::Array(Array(ty)))) => {
                Arc::new(Type::Constructor(Constructor::Value(Value::Array(Array(
                    ty.clone().follow_tvars(unifier),
                )))))
            }

            Type::Constructor(Constructor::Value(_)) => self,

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

    /// Resolves `self`, returning it as a [`crate::Type`].
    ///
    /// A resolved type is one in which all type variables have been resolved, recursively.
    ///
    /// Fails with a [`TypeError`] if the stored `Type` cannot be fully resolved.
    pub fn resolved(&self, unifier: &mut Unifier<'_>) -> Result<crate::Type, TypeError> {
        self.resolve_type(unifier)
    }

    pub(crate) fn resolved_as<T: Clone + 'static>(
        &self,
        unifier: &mut Unifier<'_>,
    ) -> Result<T, TypeError> {
        let resolved_ty: crate::Type = self.resolve_type(unifier)?;

        let result = match &resolved_ty {
            crate::Type::Constructor(crate::Constructor::Projection(projection)) => {
                if let Some(t) = (projection as &dyn std::any::Any).downcast_ref::<T>() {
                    return Ok(t.clone());
                }

                Err(())
            }
            crate::Type::Constructor(crate::Constructor::Value(value)) => {
                if let Some(t) = (value as &dyn std::any::Any).downcast_ref::<T>() {
                    return Ok(t.clone());
                }

                Err(())
            }
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
                self.clone(),
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

#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Display, Hash)]
#[display("PROJ[{}]", _0.iter().map(|pc| pc.to_string()).collect::<Vec<_>>().join(", "))]
pub struct ProjectionColumns(pub(crate) Vec<ProjectionColumn>);

/// The type of an [`sqltk::parser::ast::Expr`] or [`sqltk::parser::ast::Statement`] that returns a projection.
///
/// It represents an ordered list of zero or more optionally aliased columns types.
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord, Clone, Display, Hash)]
pub enum Projection {
    /// A projection with columns
    #[display("{}", _0)]
    WithColumns(ProjectionColumns),

    /// A projection without columns.
    ///
    /// An `INSERT`, `UPDATE` or `DELETE` statement without a `RETURNING` clause will have an empty projection.
    ///
    /// Also statements such as `SELECT FROM users` where there are no selected columns or wildcards will have an empty
    /// projection.
    #[display("PROJ[]")]
    Empty,
}

impl Projection {
    pub fn new(columns: Vec<ProjectionColumn>) -> Self {
        if columns.is_empty() {
            Projection::Empty
        } else {
            Projection::WithColumns(ProjectionColumns(Vec::from_iter(columns.iter().cloned())))
        }
    }

    pub(crate) fn flatten(&self) -> Self {
        match self {
            Projection::WithColumns(projection_columns) => {
                Projection::WithColumns(projection_columns.flatten())
            }
            Projection::Empty => Projection::Empty,
        }
    }

    pub(crate) fn len(&self) -> usize {
        match self {
            Projection::WithColumns(projection_columns) => projection_columns.len(),
            Projection::Empty => 0,
        }
    }

    pub(crate) fn columns(&self) -> &[ProjectionColumn] {
        match self {
            Projection::WithColumns(projection_columns) => projection_columns.0.as_slice(),
            Projection::Empty => &[],
        }
    }
}

impl Index<usize> for Projection {
    type Output = ProjectionColumn;

    fn index(&self, index: usize) -> &Self::Output {
        match self {
            Projection::WithColumns(projection_columns) => &projection_columns.0[index],
            Projection::Empty => panic!("cannot index into an empty projection"),
        }
    }
}

impl ProjectionColumns {
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    pub(crate) fn flatten(&self) -> Self {
        ProjectionColumns(self.flatten_impl(Vec::with_capacity(self.len())))
    }

    fn flatten_impl(&self, mut output: Vec<ProjectionColumn>) -> Vec<ProjectionColumn> {
        for ProjectionColumn { ty, alias } in &self.0 {
            match &**ty {
                Type::Constructor(Constructor::Projection(Projection::WithColumns(nested))) => {
                    output = nested.flatten_impl(output);
                }
                _ => output.push(ProjectionColumn::new(ty.clone(), alias.clone())),
            }
        }
        output
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

impl ProjectionColumns {
    pub(crate) fn new_from_schema_table(table: Arc<Table>) -> Self {
        let cols = ProjectionColumns(
            table
                .columns
                .iter()
                .map(|col| {
                    let tc = TableColumn {
                        table: table.name.clone(),
                        column: col.name.clone(),
                    };

                    let value_ty = match &col.kind {
                        ColumnKind::Native => Type::Constructor(Constructor::Value(Value::Native(
                            NativeValue(Some(tc)),
                        ))),
                        ColumnKind::Eql(features) => Type::Constructor(Constructor::Value(
                            Value::Eql(EqlTerm::Full(EqlValue(tc, *features))),
                        )),
                    };

                    ProjectionColumn::new(value_ty, Some(col.name.clone()))
                })
                .collect(),
        );

        cols
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
impl_from_for_arc_type!(Constructor);
impl_from_for_arc_type!(Value);
impl_from_for_arc_type!(Array);
impl_from_for_arc_type!(AssociatedType);

impl From<AssociatedType> for Type {
    fn from(associated: AssociatedType) -> Self {
        Type::Associated(associated)
    }
}

impl From<Constructor> for Type {
    fn from(constructor: Constructor) -> Self {
        Type::Constructor(constructor)
    }
}

impl From<Value> for Type {
    fn from(value: Value) -> Self {
        Type::Constructor(Constructor::Value(value))
    }
}

impl From<EqlTerm> for Type {
    fn from(eql_term: EqlTerm) -> Self {
        Type::Constructor(Constructor::Value(Value::Eql(eql_term)))
    }
}

impl From<Var> for Type {
    fn from(var: Var) -> Self {
        Type::Var(var)
    }
}

impl From<Projection> for Type {
    fn from(projection: Projection) -> Self {
        Type::Constructor(Constructor::Projection(projection))
    }
}

impl From<NativeValue> for Type {
    fn from(native: NativeValue) -> Self {
        Type::Constructor(Constructor::Value(Value::Native(native)))
    }
}

impl From<Array> for Type {
    fn from(array: Array) -> Self {
        Type::Constructor(Constructor::Value(Value::Array(array)))
    }
}
