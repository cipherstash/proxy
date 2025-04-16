//! Types that model the type system used by EQL Mapper.
//!
//! This is the publicly exported representation of the type system useful for crates that consume `eql_mapper`.
//!
//! `eql_mapper`'s internal representation of the type system contains additional implementation details which would not
//! be pleasant for public consumption.

use crate::{
    unifier::{EqlValue, NativeValue},
};
use derive_more::Display;
use sqlparser::ast::Ident;

/// The resolved type of a [`sqlparser::ast::Expr`] node.
#[derive(Debug, Clone, PartialEq, Eq, Display)]
pub enum Type {
    /// A value type (an EQL type, native database type or an array type)
    Value(Value),

    /// A projection type that is parameterized by a list of projection column types.
    #[display("Projection({})", _0)]
    Projection(Projection),
}

/// A value type (an EQL type, native database type or an array type)
#[derive(Debug, Clone, PartialEq, Eq, Display)]
pub enum Value {
    /// An encrypted type from a particular table-column in the schema.
    ///
    /// An encrypted column never shares a type with another encrypted column - which is why it is sufficient to
    /// identify the type by its table & column names.
    #[display("Eql({})", _0.to_string())]
    Eql(EqlValue),

    /// A native database type.
    #[display("Native")]
    Native(NativeValue),

    /// An array type that is parameterized by an element type.
    #[display("Array({})", _0)]
    Array(Box<Type>),
}

/// A projection type that is parameterized by a list of projection column types.
#[derive(Debug, Clone, PartialEq, Eq, Display)]
pub enum Projection {
    #[display("Projection::WithColumns({})", ProjectionColumn::render_projection(_0))]
    WithColumns(Vec<ProjectionColumn>),

    #[display("Projection::Empty")]
    Empty,
}

impl Projection {
    pub fn new(columns: Vec<ProjectionColumn>) -> Self {
        if columns.is_empty() {
            Projection::Empty
        } else {
            Projection::WithColumns(columns)
        }
    }
}

/// A column from a projection which has a type and an optional alias.
#[derive(Debug, PartialEq, Eq, Clone, Display)]
#[display("{} {}", ty, self.render_alias())]
pub struct ProjectionColumn {
    /// The value type of the column
    pub ty: Value,

    /// The columm alias
    pub alias: Option<Ident>,
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
