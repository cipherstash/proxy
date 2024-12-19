use std::{ops::Add, sync::Arc};

use derive_more::Display;
use sqlparser::ast::Ident;

use crate::{inference::TypeError, ColumnKind, Table};

use super::TypeRegistry;

/// The type of an expression in a SQL statement or the type of a table column from the database schema.
///
/// An expression can be:
///
/// - a [`sqlparser::ast::Expr`] node
/// - a [`sqlparser::ast::Statement`] or any other SQL AST node that produces a projection.
///
/// A `Type` is either a [`Constructor`] (fully or partially known type) or a [`TypeVar`] (a placeholder for an unknown type).
///
#[derive(Debug, PartialEq, Eq, Clone, Display)]
#[display("{self}")]
pub enum Type {
    /// A specific type constructor with zero or more generic parameters.
    #[display("Constructor({_0})")]
    Constructor(Constructor),

    /// A type variable representing a placeholder for an unknown type.
    #[display("Var({})", _0)]
    Var(TypeVar),
    // TODO: consider including `Error` as a variant
}

/// A `Constructor` is what is known about a [`Type`].
#[derive(Debug, Clone, PartialEq, Eq, Display)]
pub enum Constructor {
    #[display("Value({})", _0)]
    Value(Value),

    /// A projection type that is parameterized by a list of projection column types.
    #[display("Projection({})", crate::unifier::Unifier::render_projection(_0))]
    Projection(ProjectionColumns),

    /// An empty type - the only usecase for this type (so far) is for representing the type of subqueries that do not
    /// return a projection.
    #[display("Empty")]
    Empty,
}

#[derive(Debug, Clone, PartialEq, Eq, Display)]
pub enum Value {
    /// An encrypted type from a particular table-column in the schema.
    ///
    /// An encrypted column never shares a type with another encrypted column - which is why it is sufficient to
    /// identify the type by its table & column names.
    #[display("EQL({_0})")]
    Eql(EqlValue),

    /// A native database type that carries its table & column name.  `Native` & `AnonymousNative` are will successfully
    /// unify with each other - they are the same type as far as the type system is concerned. `Native` just carries more
    /// information which makes testing & debugging easier.
    #[display("Native")]
    Native(NativeValue),

    /// An array type that is parameterized by an element type.
    #[display("Array({})", _0)]
    Array(Box<Type>),
}

#[derive(Debug, Clone, PartialEq, Eq, Display)]
#[display("{}.{}", table, column)]
pub struct TableColumn {
    pub table: Ident,
    pub column: Ident,
}

#[derive(Debug, Clone, PartialEq, Eq, Display)]
pub struct EqlValue(pub TableColumn);

#[derive(Debug, Clone, PartialEq, Eq, Display)]
#[display("Native({})", _0.as_ref().map(|tc| tc.to_string()).unwrap_or(String::from("?")))]
pub struct NativeValue(pub Option<TableColumn>);

/// A column from a projection.
#[derive(Debug, PartialEq, Eq, Clone, Display)]
#[display("{} {}", ty, self.render_alias())]
pub struct ProjectionColumn {
    /// The type of the column
    pub ty: Type,

    /// The columm alias
    pub alias: Option<Ident>,
}

/// A placeholder for an unknown type.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Display, Default)]
pub struct TypeVar(pub u32);

/// A `Status` represents the "completeness" of a [`Type`].
#[derive(Debug, PartialEq, Eq, Copy, Clone, Display)]
pub enum Status {
    /// The type is completely known.
    ///
    /// There are no type variables (i.e. `Constructor::Var` values) contained within the type or any type it references.
    Resolved,

    /// There *might* be unresolved type variables (`Constructor::Var`) contained within the type.
    ///
    /// It is possible that all the types contained by a type have since been resolved but because the unification
    /// algorithm works on a directed acyclic graph which permits multiple paths to a single type it is possible for all
    /// child nodes of a type to become resolved without that information being propagated back to all types that
    /// reference it.
    ///
    /// When a `Type` claims to be `Partial` but a fully resolved type is required, call [`Type::try_resolve`] to refresh
    /// its status.
    Partial,
}

impl TypeVar {
    pub(crate) fn try_resolve(&self, registry: &TypeRegistry<'_>) -> Result<Type, TypeError> {
        let mut tvar = *self;
        loop {
            match registry.get_substitution(tvar) {
                Some((ty @ Type::Constructor(_), Status::Resolved)) => return Ok(ty.clone()),
                Some((ty @ Type::Constructor(_), Status::Partial)) => {
                    return ty.try_resolve(registry)
                }
                Some((ty @ Type::Var(_), Status::Resolved)) => return ty.try_resolve(registry),
                Some((Type::Var(found), Status::Partial)) => tvar = found,
                None => {
                    return Err(TypeError::Incomplete(format!(
                        "{} has no inferred substitution",
                        tvar
                    )))
                }
            }
        }
    }

    pub(crate) fn is_fully_resolved(&self, registry: &TypeRegistry<'_>) -> bool {
        self.try_resolve(registry).is_ok()
    }
}

impl Add for Status {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        if let (Self::Resolved, Self::Resolved) = (self, rhs) {
            return Self::Resolved;
        }

        Self::Partial
    }
}

impl Type {
    /// Creates an `Type` containing a `Constructor::Empty`.
    pub(crate) fn empty() -> Self {
        Type::Constructor(Constructor::Empty)
    }

    /// Creates an `Type` containing a `Constructor::Scalar(Scalar::Native(None))`.
    pub(crate) fn any_native() -> Self {
        Type::Constructor(Constructor::Value(Value::Native(NativeValue(None))))
    }

    /// Creates an `Type` containing a `Constructor::Projection`.
    pub(crate) fn projection(columns: &[(Type, Option<Ident>)]) -> Self {
        Type::Constructor(Constructor::Projection(ProjectionColumns(
            columns
                .iter()
                .map(|(c, n)| ProjectionColumn::new(c.clone(), n.clone()))
                .collect(),
        )))
    }

    /// Creates an `Type` containing a `Constructor::Array`.
    pub(crate) fn array(element_ty: Type) -> Self {
        Type::Constructor(Constructor::Value(Value::Array(element_ty.into())))
    }

    /// Gets the status of this type.
    pub(crate) fn is_fully_resolved(&self, registry: &TypeRegistry<'_>) -> bool {
        match self {
            Type::Constructor(constructor) => constructor.is_fully_resolved(registry),
            Type::Var(tvar) => tvar.is_fully_resolved(registry),
        }
    }

    pub(crate) fn status(&self, registry: &TypeRegistry<'_>) -> Status {
        if self.is_fully_resolved(registry) {
            Status::Resolved
        } else {
            Status::Partial
        }
    }

    /// Tries to resolve this type.
    ///
    /// See [`Status::Partial`] for an explanation of why this method is required.
    pub(crate) fn try_resolve(&self, registry: &TypeRegistry) -> Result<Type, TypeError> {
        match &self {
            Self::Constructor(constructor) => Ok(Type::Constructor(
                constructor.try_resolve(registry)?.clone(),
            )),

            Self::Var(tvar) => Ok(tvar.try_resolve(registry)?),
        }
    }
}

impl Constructor {
    /// Tries to resolve all type variables recursively referenced by this type.
    ///
    /// See [`Status::Partial`] for a complete explanation of why this is required.
    fn try_resolve(&self, registry: &TypeRegistry<'_>) -> Result<Self, TypeError> {
        match self {
            Constructor::Value(Value::Array(element_ty)) => Ok(Constructor::Value(Value::Array(
                element_ty.try_resolve(registry)?.into(),
            ))),

            Constructor::Value(_) => Ok(self.clone()),

            Constructor::Projection(ProjectionColumns(cols)) => {
                Ok(Constructor::Projection(ProjectionColumns(
                    cols.iter()
                        .map(|col| col.try_resolve(registry))
                        .collect::<Result<Vec<_>, _>>()?,
                )))
            }

            Constructor::Empty => Ok(Constructor::Empty),
        }
    }

    fn is_fully_resolved(&self, registry: &TypeRegistry<'_>) -> bool {
        match self {
            Constructor::Value(Value::Array(element_ty)) => element_ty.is_fully_resolved(registry),
            Constructor::Value(_) => true,
            Constructor::Projection(ProjectionColumns(cols)) => {
                cols.iter().all(|col| col.ty.is_fully_resolved(registry))
            }
            Constructor::Empty => true,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Display)]
#[display("[{}]", _0.iter().map(|pc| pc.to_string()).collect::<Vec<_>>().join(", "))]
pub struct ProjectionColumns(pub(crate) Vec<ProjectionColumn>);

impl ProjectionColumns {
    pub(crate) fn len(&self) -> usize {
        self.0.len()
    }

    pub(crate) fn flatten(&self) -> Self {
        let output: Vec<ProjectionColumn> = Vec::with_capacity(self.len());
        ProjectionColumns(Self::flatten_impl(self, output))
    }

    fn flatten_impl(&self, mut output: Vec<ProjectionColumn>) -> Vec<ProjectionColumn> {
        for ProjectionColumn { ty, alias } in &self.0 {
            match &ty {
                Type::Constructor(Constructor::Projection(nested)) => {
                    output = Self::flatten_impl(nested, output);
                }
                other => output.push(ProjectionColumn {
                    ty: (*other).clone(),
                    alias: alias.clone(),
                }),
            }
        }
        output
    }
}

impl From<Arc<Table>> for ProjectionColumns {
    fn from(table: Arc<Table>) -> Self {
        ProjectionColumns(
            table
                .columns
                .iter()
                .map(|col| {
                    let tc = TableColumn {
                        table: table.name.clone(),
                        column: col.name.clone(),
                    };

                    let value_ty = if col.kind == ColumnKind::Native {
                        Value::Native(NativeValue(Some(tc)))
                    } else {
                        Value::Eql(EqlValue(tc))
                    };

                    ProjectionColumn::new(
                        Type::Constructor(Constructor::Value(value_ty)),
                        Some(col.name.clone()),
                    )
                })
                .collect(),
        )
    }
}

impl ProjectionColumn {
    pub(crate) fn new(ty: Type, alias: Option<Ident>) -> Self {
        Self { ty, alias }
    }

    fn render_alias(&self) -> String {
        match &self.alias {
            Some(name) => name.to_string(),
            None => String::from("(no-alias)"),
        }
    }

    fn try_resolve(&self, registry: &TypeRegistry<'_>) -> Result<ProjectionColumn, TypeError> {
        Ok(ProjectionColumn {
            ty: self.ty.try_resolve(registry)?,
            alias: self.alias.clone(),
        })
    }
}
