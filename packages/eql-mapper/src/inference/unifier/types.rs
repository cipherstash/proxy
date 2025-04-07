use std::{ops::Index, sync::Arc};

use derive_more::Display;
use sqlparser::ast::Ident;

use crate::{ColumnKind, Table};

use super::TypeCell;

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

const _: () = {
    fn assert_send<T: Send>() {}
    fn assert_sync<T: Sync>() {}

    // RFC 2056
    fn assert_all() {
        assert_send::<Type>();
        assert_sync::<Type>();
    }
};

/// A `Constructor` is what is known about a [`Type`].
#[derive(Debug, Clone, PartialEq, Eq, Display)]
pub enum Constructor {
    #[display("Value({})", _0)]
    Value(Value),

    /// A projection type that is parameterized by a list of projection column types.
    #[display("Projection({})", _0)]
    Projection(Projection),
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
    #[display("Array({})", *_0.as_type())]
    Array(TypeCell),
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
#[display("{} {}", *ty.as_type(), self.render_alias())]
pub struct ProjectionColumn {
    /// The type of the column.
    pub ty: TypeCell,

    /// The columm alias
    pub alias: Option<Ident>,
}

/// A placeholder for an unknown type.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Display, Default)]
pub struct TypeVar(pub u32);

impl Type {
    pub(crate) fn into_type_cell(self) -> TypeCell {
        TypeCell::new(self)
    }

    /// Creates an `Type` containing an empty projection
    pub(crate) fn empty_projection() -> TypeCell {
        Type::Constructor(Constructor::Projection(Projection::Empty)).into_type_cell()
    }

    /// Creates an `Type` containing a `Constructor::Scalar(Scalar::Native(None))`.
    pub(crate) fn any_native() -> TypeCell {
        Type::Constructor(Constructor::Value(Value::Native(NativeValue(None)))).into_type_cell()
    }

    /// Creates an `Type` containing a `Constructor::Projection`.
    pub(crate) fn projection(columns: &[(TypeCell, Option<Ident>)]) -> TypeCell {
        if columns.is_empty() {
            Type::Constructor(Constructor::Projection(Projection::Empty)).into_type_cell()
        } else {
            Type::Constructor(Constructor::Projection(Projection::WithColumns(
                ProjectionColumns(
                    columns
                        .iter()
                        .map(|(c, n)| ProjectionColumn::new(c.clone(), n.clone()))
                        .collect(),
                ),
            )))
            .into_type_cell()
        }
    }

    /// Creates an `Type` containing a `Constructor::Array`.
    pub(crate) fn array(element_ty: TypeCell) -> TypeCell {
        Type::Constructor(Constructor::Value(Value::Array(element_ty))).into_type_cell()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Display)]
#[display("[{}]", _0.iter().map(|pc| pc.to_string()).collect::<Vec<_>>().join(", "))]
pub struct ProjectionColumns(pub(crate) Vec<ProjectionColumn>);

/// The type of an [`sqlparser::ast::Expr`] or [`sqlparser::ast::Statement`] that returns a projection.
///
/// It represents an ordered list of zero or more optionally aliased columns types.
#[derive(Debug, Clone, PartialEq, Eq, Display)]
pub enum Projection {
    /// A projection with columns
    #[display("Projection::Columns({})", _0)]
    WithColumns(ProjectionColumns),

    /// A projection without columns.
    ///
    /// An `INSERT`, `UPDATE` or `DELETE` statement without a `RETURNING` clause will have an empty projection.
    ///
    /// Also statements such as `SELECT FROM users` where there are no selected columns or wildcards will have an empty
    /// projection.
    #[display("Projection::Empty")]
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
        for ProjectionColumn {
            ty,
            alias,
        } in &self.0
        {
            match &*ty.as_type() {
                Type::Constructor(Constructor::Projection(Projection::WithColumns(nested))) => {
                    output = nested.flatten_impl(output);
                }
                _ => output.push(ProjectionColumn::new(ty.clone(), alias.clone())),
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
                        Type::Constructor(Constructor::Value(value_ty)).into_type_cell(),
                        Some(col.name.clone()),
                    )
                })
                .collect(),
        )
    }
}

impl ProjectionColumn {
    /// Returns a new `ProjectionColumn` with type `ty` and optional `alias`.
    pub(crate) fn new(ty: TypeCell, alias: Option<Ident>) -> Self {
        Self {
            ty,
            alias,
        }
    }

    fn render_alias(&self) -> String {
        match &self.alias {
            Some(name) => name.to_string(),
            None => String::from("(no-alias)"),
        }
    }
}
