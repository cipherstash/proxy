use std::{cell::RefCell, ops::Add, rc::Rc, sync::Arc};

use derive_more::Display;
use sqlparser::ast::Ident;

use crate::{inference::TypeError, Table};

/// The inferred type of either:
///
/// - an `Expr` node, or
/// - any SQL statement or subquery that produces a projection
/// - a table-column from the database schema
///
/// A `Type` has a [`Def`] and a [`Status`]. The `Def` contains the inferred details of the type.  The `Status` captures
/// whether the type is fully resolved or partial (may contain unsubstituted type variables).
///
#[derive(Debug, PartialEq, Eq, Clone, Display)]
#[display("Type({_0}, {_1})")]
pub struct Type(pub(crate) Def, pub(crate) Status);

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
    pub fn new(def: Def) -> Self {
        let status = match &def {
            Def::Constructor(constructor) => match constructor {
                Constructor::Scalar(_) => Status::Resolved,
                Constructor::Array(element_ty) => element_ty.borrow().status(),
                Constructor::Projection(columns) => {
                    if columns
                        .borrow()
                        .iter()
                        .map(|col| match &*col.ty.borrow() {
                            Type(Def::Constructor(Constructor::Projection(_)), _) => {
                                Status::Partial
                            }
                            Type(_, status) => *status,
                        })
                        .all(|s| s == Status::Resolved)
                    {
                        Status::Resolved
                    } else {
                        Status::Partial
                    }
                }
                Constructor::Empty => Status::Resolved,
            },
            Def::Var(_) => Status::Partial,
        };

        Self(def, status)
    }

    /// Creates an `Rc<RefCell<Type>>` containing a `Constructor::Empty`.
    pub fn empty() -> Rc<RefCell<Self>> {
        Self::new(Def::Constructor(Constructor::Empty)).wrap()
    }

    /// Creates an `Rc<RefCell<Type>>` containing a `Constructor::Scalar(Arc::new(Scalar::AnonymousNative))`.
    pub fn anonymous_native() -> Rc<RefCell<Self>> {
        Self::new(Def::Constructor(Constructor::Scalar(Arc::new(
            Scalar::AnonymousNative,
        ))))
        .wrap()
    }

    /// Creates an `Rc<RefCell<Type>>` containing a `Constructor::Scalar(Scalar::Encrypted{ .. })`.
    pub fn encrypted_scalar<T: Into<Ident>, C: Into<Ident>>(
        table: T,
        column: C,
    ) -> Rc<RefCell<Self>> {
        Self::new(Def::Constructor(Constructor::Scalar(Arc::new(
            Scalar::Encrypted {
                table: table.into(),
                column: column.into(),
            },
        ))))
        .wrap()
    }

    /// Creates an `Rc<RefCell<Type>>` containing a `Constructor::Scalar(Scalar::Encrypted{ .. })`.
    pub fn native_scalar<T: Into<Ident>, C: Into<Ident>>(table: T, column: C) -> Rc<RefCell<Self>> {
        Self::new(Def::Constructor(Constructor::Scalar(Arc::new(
            Scalar::Native {
                table: table.into(),
                column: column.into(),
            },
        ))))
        .wrap()
    }

    /// Creates an `Rc<RefCell<Type>>` containing a `TypeVar::Fresh`.
    pub fn fresh_tvar() -> Rc<RefCell<Self>> {
        Self::new(Def::Var(TypeVar::Fresh)).wrap()
    }

    /// Creates an `Rc<RefCell<Type>>` containing a `Constructor::Projection`.
    pub fn projection(columns: &[(Rc<RefCell<Type>>, Option<Ident>)]) -> Rc<RefCell<Self>> {
        Self::new(Def::Constructor(Constructor::Projection(Rc::new(
            RefCell::new(
                columns
                    .iter()
                    .map(|(c, n)| ProjectionColumn::new(c.clone(), n.clone()))
                    .collect(),
            ),
        ))))
        .wrap()
    }

    /// Creates an `Rc<RefCell<Type>>` containing a `Constructor::Array`.
    pub fn array(element_ty: Rc<RefCell<Type>>) -> Rc<RefCell<Self>> {
        Self::new(Def::Constructor(Constructor::Array(element_ty))).wrap()
    }

    /// Creates an `Rc<RefCell<Type>>` containing a `Constructor::Scalar`.
    pub fn scalar(scalar_ty: Arc<Scalar>) -> Rc<RefCell<Self>> {
        Self::new(Def::Constructor(Constructor::Scalar(scalar_ty))).wrap()
    }

    /// Wraps `self` in an `Rc<RefCell<_>>`.
    ///
    /// Convenience to avoid boilerplate.
    pub fn wrap(self) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(self))
    }

    /// Checks if this type is fully resolved (contains no type variables),
    pub fn is_resolved(&self) -> bool {
        self.1 == Status::Resolved
    }

    /// Gets the status of this type.
    pub fn status(&self) -> Status {
        self.1
    }

    /// Tries to resolve this type.
    ///
    /// See [`Status::Partial`] for an explanation of why this method is required.
    pub fn try_resolve(&mut self) -> Result<(), TypeError> {
        if self.is_resolved() {
            return Ok(());
        }

        match &mut self.0 {
            Def::Constructor(constructor) => {
                constructor.try_resolve()?;
                self.1 = Status::Resolved;
                Ok(())
            }

            Def::Var(_) => Err(TypeError::Incomplete),
        }
    }
}

/// A `Def` is either a [`Constructor`] (fully or partially known type) or a [`TypeVar`] (a placeholder for an unknown type).
#[derive(Debug, PartialEq, Eq, Clone, Display)]
#[display("{self}")]
pub enum Def {
    /// A specific type constructor with zero or more generic parameters.
    #[display("Constructor({_0})")]
    Constructor(Constructor),

    /// A type variable representing a placeholder for an unknown type.
    #[display("Var({_0})")]
    Var(TypeVar),
}

/// A `Constructor` is what is known about a [`Type`].
#[derive(Debug, Clone, PartialEq, Eq, Display)]
pub enum Constructor {
    /// A [`Scalar`] type; either an encrypted column from the database schema or some native (plaintext) database type.
    #[display("Scalar({_0})")]
    Scalar(Arc<Scalar>),

    /// An array type that is parameterized by an element type.
    #[display("Array({})", _0.borrow())]
    Array(Rc<RefCell<Type>>),

    /// A projection type that is parameterized by a list of projection column types.
    #[display("Projection({})", crate::Unifier::render_projection(_0.clone()))]
    Projection(Rc<RefCell<Vec<ProjectionColumn>>>),

    /// An empty type - the only usecase for this type (so far) is for representing the type of subqueries that do not
    /// return a projection.
    #[display("Empty")]
    Empty,
}

impl Constructor {
    pub fn is_native(&self) -> bool {
        match self {
            Constructor::Scalar(s) => {
                matches!(&**s, Scalar::Native { .. } | Scalar::AnonymousNative)
            }
            _ => false,
        }
    }
}

impl From<&Table> for Vec<ProjectionColumn> {
    fn from(table: &Table) -> Self {
        table
            .columns
            .iter()
            .map(|col| {
                ProjectionColumn::new(
                    Type(
                        Def::Constructor(Constructor::Scalar(col.ty.clone())),
                        Status::Resolved,
                    )
                    .wrap(),
                    Some(col.name.clone()),
                )
            })
            .collect()
    }
}

impl Constructor {
    /// Tries to resolve all type variables recursively referenced by this type.
    ///
    /// See [`Status::Partial`] for a complete explanation of why this is required.
    fn try_resolve(&self) -> Result<(), TypeError> {
        match self {
            Constructor::Scalar(_) => Ok(()),

            Constructor::Array(element_ty) => {
                let ty = &mut *element_ty.borrow_mut();
                ty.try_resolve()
            }

            Constructor::Projection(columns) => {
                let columns = &*columns.borrow();
                for column in columns {
                    let ty = &mut *column.ty.borrow_mut();
                    ty.try_resolve()?;
                }
                Ok(())
            }

            Constructor::Empty => Ok(()),
        }
    }
}

/// The type of an encrypted column or a native (plaintext) database types.
///
/// Native database types are not distinguished in this type system. Valid usage of native types is best determined by
/// the database.
#[derive(Debug, Clone, Eq, PartialEq, PartialOrd, Ord, Display, Hash)]
pub enum Scalar {
    /// An encrypted type from a particular table-column in the schema.
    ///
    /// An encrypted column never shares a type with another encrypted column - which is why it is sufficient to
    /// identify the type by its table & column names.
    #[display("Encrypted({table}.{column})")]
    Encrypted { table: Ident, column: Ident },

    /// A native database type that carries its table & column name.  `Native` & `AnonymousNative` are will successfully
    /// unify with each other - they are the same type as far as the type system is concerned. `Native` just carries more
    /// information which makes testing & debugging easier.
    #[display("Native")]
    Native { table: Ident, column: Ident },

    /// Any other type, such as a native plaintext database type
    #[display("AnonymousNative")]
    AnonymousNative,
}

/// A column from a projection.
#[derive(Debug, PartialEq, Eq, Clone, Display)]
#[display("{} {}", ty.borrow(), self.render_alias())]
pub struct ProjectionColumn {
    /// The type of the column
    pub ty: Rc<RefCell<Type>>,

    /// The columm alias
    pub alias: Option<Ident>,
}

impl ProjectionColumn {
    pub(crate) fn new(ty: Rc<RefCell<Type>>, alias: Option<Ident>) -> Self {
        Self { ty, alias }
    }

    fn render_alias(&self) -> String {
        match &self.alias {
            Some(name) => name.to_string(),
            None => String::from("(no-alias)"),
        }
    }
}

/// A placeholder for an unknown type.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Display)]
pub enum TypeVar {
    /// A type variable that has not yet been assigned a unique identifier.
    Fresh,

    /// A type variable with an identifier.
    Assigned(u32),
}
