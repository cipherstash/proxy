//! Type definitions for consumers of `eql_mapper`.
//!
//! These definitions can only represent fully-resolved types after type checking and therefore there is no need for
//! tracking a resolution status ([`crate::Status`]), type variables ([`crate::TypeVar`]) etc. In addition, projections
//! are fully flattened (thus simplifying their columns types to just scalars and arrays).
//!
//! Additionally, all indirection via `Rc<RefCell<_>>` and `Arc<_>` has been removed.

use derive_more::Display;
use sqlparser::ast::Ident;

use crate::{Constructor, Def, EqlMapperError, Status};

/// The resolved type of a [`sqlparser::ast::Expr`] node.
#[derive(Debug, Clone, PartialEq, Eq, Display)]
pub enum Type {
    /// A [`Scalar`] type; either an encrypted column from the database schema or some native (plaintext) database type.
    #[display("Scalar({_0})")]
    Scalar(Scalar),

    /// An array type that is parameterized by an element type.
    #[display("Array({})", _0)]
    Array(Box<Type>),

    /// A projection type that is parameterized by a list of projection column types.
    #[display("Projection({})", ProjectionColumn::render_projection(_0.0.as_slice()))]
    Projection(Projection),

    /// An empty type - the only use case for this type (so far) is for representing the type of subqueries that do not
    /// return a projection.
    #[display("Empty")]
    Empty,
}

/// A projection type that is parameterized by a list of projection column types.
#[derive(Debug, Clone, PartialEq, Eq, Display)]
#[display("Projection{}", ProjectionColumn::render_projection(_0))]
pub struct Projection(pub Vec<ProjectionColumn>);

/// The type of an encrypted column or a native (plaintext) database types.
///
/// Native database types are not distinguished in this type system and the [`PartialEq`] impl for `Scalar` ignores the
/// optional [`TableColumn`].
#[derive(Debug, Clone, Eq, Display)]
pub enum Scalar {
    /// An encrypted type from a particular table-column in the schema.
    ///
    /// An encrypted column never shares a type with another encrypted column - which is why it is sufficient to
    /// identify the type by its table & column names.
    #[display("EQL({}.{})", _0.table.to_string(), _0.column.to_string())]
    EqlColumn(TableColumn),

    /// A native database type.
    #[display("Native")]
    Native(Option<TableColumn>),
}

/// A reference to a table and a column. This can be used to resolve type information in a [`crate::Schema`].
#[derive(Debug, Clone, Eq, PartialEq, Display)]
#[display("{}.{}", table.to_string(), column.to_string())]
pub struct TableColumn {
    pub table: Ident,
    pub column: Ident,
}

/// A column from a projection which has a type and an optional alias.
#[derive(Debug, PartialEq, Eq, Clone, Display)]
#[display("{} {}", ty, self.render_alias())]
pub struct ProjectionColumn {
    /// The type of the column
    pub ty: ProjectionColumnType,

    /// The columm alias
    pub alias: Option<Ident>,
}

/// The type of a projection column. Projections produced from wildcards have been flattened, so projections cannot
/// contain projections.  The only possible kinds of types for columns are arrays and scalars.
#[derive(Debug, PartialEq, Eq, Clone, Display)]
pub enum ProjectionColumnType {
    #[display("{}", _0)]
    Scalar(Scalar),

    #[display("{}", _0)]
    Array(Box<ProjectionColumnType>),
}

impl ProjectionColumn {
    fn render_alias(&self) -> String {
        match &self.alias {
            Some(name) => name.to_string(),
            None => String::from("(no-alias)"),
        }
    }

    fn render_projection(projection: &[ProjectionColumn]) -> String {
        let ty_strings: Vec<String> = projection.iter().map(|col| col.ty.to_string()).collect();
        format!("[{}]", ty_strings.join(", "))
    }
}

impl TryFrom<&crate::Type> for Type {
    type Error = EqlMapperError;

    fn try_from(value: &crate::Type) -> Result<Self, Self::Error> {
        let crate::Type(Def::Constructor(constructor), Status::Resolved) = value else {
            return Err(EqlMapperError::InternalError(format!(
                "expected type {} to be resolved",
                value
            )));
        };

        match constructor {
            crate::Constructor::Scalar(ty) => (&**ty).try_into(),

            crate::Constructor::Array(ty) => (&*ty.borrow()).try_into(),

            crate::Constructor::Projection(ty) => {
                let columns = &*ty.borrow();
                let mut pub_columns: Vec<ProjectionColumn> = Vec::with_capacity(columns.len());

                for crate::ProjectionColumn { ty, alias } in columns.iter() {
                    let pub_column: ProjectionColumn = match &*ty.borrow() {
                        crate::Type(Def::Constructor(Constructor::Scalar(ty)), _) => {
                            ProjectionColumn {
                                ty: (&**ty).try_into()?,
                                alias: alias.clone(),
                            }
                        }

                        crate::Type(Def::Constructor(Constructor::Array(ty)), _) => {
                            ProjectionColumn {
                                ty: (&*ty.borrow()).try_into()?,
                                alias: alias.clone(),
                            }
                        }

                        unexpected => Err(EqlMapperError::InternalError(format!(
                            "unexpected type {} in projection column",
                            unexpected
                        )))?,
                    };

                    pub_columns.push(pub_column);
                }

                Ok(Type::Projection(Projection(pub_columns)))
            }

            crate::Constructor::Empty => Ok(Type::Empty),
        }
    }
}

impl TryFrom<&crate::Scalar> for Type {
    type Error = EqlMapperError;

    fn try_from(scalar: &crate::Scalar) -> Result<Self, Self::Error> {
        match scalar {
            crate::Scalar::Encrypted { table, column } => {
                Ok(Self::Scalar(Scalar::EqlColumn(TableColumn {
                    table: (*table).clone(),
                    column: (*column).clone(),
                })))
            }
            crate::Scalar::Native { table, column } => {
                Ok(Self::Scalar(Scalar::Native(Some(TableColumn {
                    table: (*table).clone(),
                    column: (*column).clone(),
                }))))
            }
            crate::Scalar::AnonymousNative => Ok(Self::Scalar(Scalar::Native(None))),
        }
    }
}

impl TryFrom<&crate::Scalar> for ProjectionColumnType {
    type Error = EqlMapperError;

    fn try_from(scalar: &crate::Scalar) -> Result<Self, Self::Error> {
        match scalar {
            crate::Scalar::Encrypted { table, column } => {
                Ok(Self::Scalar(Scalar::EqlColumn(TableColumn {
                    table: (*table).clone(),
                    column: (*column).clone(),
                })))
            }
            crate::Scalar::Native { table, column } => {
                Ok(Self::Scalar(Scalar::Native(Some(TableColumn {
                    table: (*table).clone(),
                    column: (*column).clone(),
                }))))
            }
            crate::Scalar::AnonymousNative => Ok(Self::Scalar(Scalar::Native(None))),
        }
    }
}

impl TryFrom<&crate::Type> for ProjectionColumnType {
    type Error = EqlMapperError;

    fn try_from(value: &crate::Type) -> Result<Self, Self::Error> {
        let crate::Type(Def::Constructor(constructor), Status::Resolved) = value else {
            return Err(EqlMapperError::InternalError(format!(
                "expected type {} to be resolved",
                value
            )));
        };

        match constructor {
            crate::Constructor::Scalar(ty) => (&**ty).try_into(),

            crate::Constructor::Array(ty) => (&*ty.borrow()).try_into(),

            unexpected => Err(EqlMapperError::InternalError(format!(
                "expected projection column type {} to be a scalar or an array",
                unexpected
            ))),
        }
    }
}

impl TryFrom<&crate::Scalar> for Scalar {
    type Error = EqlMapperError;

    fn try_from(scalar: &crate::Scalar) -> Result<Self, Self::Error> {
        match scalar {
            crate::Scalar::Encrypted { table, column } => Ok(Scalar::EqlColumn(TableColumn {
                table: table.clone(),
                column: column.clone(),
            })),
            crate::Scalar::Native { table, column } => Ok(Scalar::Native(Some(TableColumn {
                table: table.clone(),
                column: column.clone(),
            }))),
            crate::Scalar::AnonymousNative => Ok(Scalar::Native(None)),
        }
    }
}

impl PartialEq for Scalar {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Scalar::EqlColumn(left), Scalar::EqlColumn(right)) => left == right,
            (Scalar::Native(_), Scalar::Native(_)) => true,
            _ => false,
        }
    }
}
