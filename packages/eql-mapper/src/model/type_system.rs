//! Types that model the type system used by EQL Mapper.
//!
//! This is the publicly exported representation of the type system useful for crates that consume `eql_mapper`.
//!
//! `eql_mapper`'s internal representation of the type system contains additional implementation details which would not
//! be pleasant for public consumption.

use crate::unifier::{EqlTerm, NativeValue};
use derive_more::Display;
use sqltk::parser::ast::Ident;

/// The resolved type of a [`sqltk::parser::ast::Expr`] node.
#[derive(Debug, Clone, PartialEq, Eq, Display)]
pub enum Type {
    /// A value type (an EQL type, native database type or an array type)
    #[display("{}", _0)]
    Value(Value),
}

#[derive(Debug, Clone, PartialEq, Eq, Display)]
#[display("SetOf<{}>", _0)]
pub struct SetOf(pub Box<Type>);
impl SetOf {
    fn contains_eql(&self) -> bool {
        self.0.contains_eql()
    }
}

/// A value type (an EQL type, native database type or an array type)
#[derive(Debug, Clone, PartialEq, Eq, Display)]
pub enum Value {
    /// An encrypted type from a particular table-column in the schema.
    ///
    /// An encrypted column never shares a type with another encrypted column - which is why it is sufficient to
    /// identify the type by its table & column names.
    #[display("{}", _0)]
    Eql(EqlTerm),

    /// A native database type.
    #[display("{}", _0)]
    Native(NativeValue),

    /// An array type that is parameterized by an element type.
    #[display("Array[{}]", _0)]
    Array(Array),

    /// A projection type that is parameterized by a list of projection column types.
    #[display("{}", _0)]
    Projection(Projection),

    /// In PostgreSQL, SETOF is a special return type used in functions to indicate that the function returns a set of
    /// rows rather than a single value. It allows a function to behave like a table or subquery in SQL, producing
    /// multiple rows as output.
    #[display("{}", _0)]
    SetOf(SetOf),
}

#[derive(Debug, Clone, PartialEq, Eq, Display)]
pub struct Array(pub Box<Type>);

/// A projection type that is parameterized by a list of projection column types.
#[derive(Debug, Clone, PartialEq, Eq, Display)]
#[display("{{{}}}", _0.iter().map(|pc| pc.to_string()).collect::<Vec<_>>().join(", "))]
pub struct Projection(pub Vec<ProjectionColumn>);

impl Type {
    pub fn contains_eql(&self) -> bool {
        match self {
            Type::Value(value) => value.contains_eql(),
        }
    }
}

impl Value {
    pub fn contains_eql(&self) -> bool {
        match self {
            Value::Eql(_) => true,
            Value::Native(_) => false,
            Value::Array(inner) => inner.contains_eql(),
            Value::Projection(projection) => projection.contains_eql(),
            Value::SetOf(set_of) => set_of.contains_eql(),
        }
    }
}

impl Array {
    pub fn contains_eql(&self) -> bool {
        let Array(element_ty) = self;
        element_ty.contains_eql()
    }
}

impl Projection {
    pub fn new(columns: Vec<ProjectionColumn>) -> Self {
        Self(columns)
    }

    pub fn type_at_col_index(&self, index: usize) -> Option<&Value> {
        self.0.get(index).map(|col| &col.ty)
    }

    pub fn contains_eql(&self) -> bool {
        self.0.iter().any(|col| col.ty.contains_eql())
    }
}

/// A column from a projection which has a type and an optional alias.
#[derive(Debug, PartialEq, Eq, Clone, Display)]
#[display("{}{}", ty, self.render_alias())]
pub struct ProjectionColumn {
    /// The value type of the column
    pub ty: Value,

    /// The columm alias
    pub alias: Option<Ident>,
}

impl ProjectionColumn {
    fn render_alias(&self) -> String {
        match &self.alias {
            Some(name) => format!(": {}", name),
            None => String::from(""),
        }
    }
}

impl From<Value> for Type {
    fn from(value: Value) -> Self {
        Type::Value(value)
    }
}

impl From<Array> for Type {
    fn from(array: Array) -> Self {
        Type::Value(Value::Array(array))
    }
}

impl From<EqlTerm> for Type {
    fn from(eql_term: EqlTerm) -> Self {
        Type::Value(Value::Eql(eql_term))
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
