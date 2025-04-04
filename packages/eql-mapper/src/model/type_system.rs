//! Types that model the type system used by EQL Mapper.
//!
//! This is the publicly exported representation of the type system useful for crates that consume `eql_mapper`.
//!
//! `eql_mapper`'s internal representation of the type system contains additional implementation details which would not
//! be pleasant for public consumption.

use crate::{
    inference::unifier,
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
#[derive(Debug, Clone, Eq, Display)]
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

impl TryFrom<&unifier::ProjectionColumns> for Type {
    type Error = crate::EqlMapperError;

    fn try_from(columns: &unifier::ProjectionColumns) -> Result<Self, Self::Error> {
        let mut pub_columns: Vec<ProjectionColumn> = Vec::with_capacity(columns.len());
        let columns = columns.flatten();

        for unifier::ProjectionColumn { ty, alias } in columns.0.iter() {
            let pub_column: ProjectionColumn = match &*ty.as_type() {
                unifier::Type::Constructor(unifier::Constructor::Value(value)) => {
                    ProjectionColumn {
                        ty: value.try_into()?,
                        alias: alias.clone(),
                    }
                }

                unexpected => Err(crate::EqlMapperError::InternalError(format!(
                    "unexpected type {} in projection column",
                    unexpected
                )))?,
            };

            pub_columns.push(pub_column);
        }

        Ok(Type::Projection(Projection::new(pub_columns)))
    }
}

impl TryFrom<&unifier::Value> for EqlValue {
    type Error = crate::EqlMapperError;

    fn try_from(value: &unifier::Value) -> Result<Self, Self::Error> {
        match value {
            unifier::Value::Eql(eql_value) => Ok(EqlValue(eql_value.0.clone())),
            other => Err(crate::EqlMapperError::InternalError(format!(
                "cannot convert {} into Value",
                other
            ))),
        }
    }
}

impl TryFrom<&unifier::Value> for Value {
    type Error = crate::EqlMapperError;

    fn try_from(value: &unifier::Value) -> Result<Self, Self::Error> {
        match value {
            unifier::Value::Eql(eql_value) => Ok(Value::Eql(eql_value.clone())),
            unifier::Value::Native(native_value) => Ok(Value::Native(native_value.clone())),
            unifier::Value::Array(element_ty) => {
                Ok(Value::Array(Box::new((&*element_ty.as_type()).try_into()?)))
            }
        }
    }
}

impl TryFrom<&unifier::Type> for Type {
    type Error = crate::EqlMapperError;

    fn try_from(value: &unifier::Type) -> Result<Self, Self::Error> {
        let unifier::Type::Constructor(constructor) = value else {
            return Err(crate::EqlMapperError::InternalError(format!(
                "expected type {} to be resolved",
                value
            )));
        };

        match constructor {
            unifier::Constructor::Value(value) => Ok(Type::Value(value.try_into()?)),

            unifier::Constructor::Projection(unifier::Projection::WithColumns(columns)) => {
                let mut pub_columns: Vec<ProjectionColumn> = Vec::with_capacity(columns.len());
                let columns = columns.flatten();

                for unifier::ProjectionColumn { ty, alias } in columns.0.iter() {
                    let pub_column: ProjectionColumn = match &*ty.as_type() {
                        unifier::Type::Constructor(unifier::Constructor::Value(ty)) => {
                            ProjectionColumn {
                                ty: ty.try_into()?,
                                alias: alias.clone(),
                            }
                        }

                        unexpected => Err(crate::EqlMapperError::InternalError(format!(
                            "unexpected type {} in projection column",
                            unexpected
                        )))?,
                    };

                    pub_columns.push(pub_column);
                }

                Ok(Type::Projection(Projection::new(pub_columns)))
            }

            unifier::Constructor::Projection(unifier::Projection::Empty) => {
                Ok(Type::Projection(Projection::Empty))
            }
        }
    }
}

impl TryFrom<&unifier::Value> for ProjectionColumn {
    type Error = crate::EqlMapperError;

    fn try_from(value: &unifier::Value) -> Result<Self, Self::Error> {
        Ok(ProjectionColumn {
            ty: value.try_into()?,
            alias: None,
        })
    }
}

impl PartialEq for Value {
    fn eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Value::Eql(lhs), Value::Eql(rhs)) => lhs == rhs,
            (Value::Native(_), Value::Native(_)) => true,
            (Value::Array(lhs), Value::Array(rhs)) => lhs == rhs,
            (_, _) => false,
        }
    }
}
