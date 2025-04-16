use std::{any::type_name, ops::Index, sync::Arc};

use derive_more::Display;
use sqlparser::ast::{self, Ident};

use crate::{ColumnKind, Table, TypeError, TypeRegistry, TID};

use super::Unifier;

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
#[derive(Debug, Clone, PartialEq, Eq, Display)]
pub enum Constructor {
    #[display("Value({})", _0)]
    Value(Value),

    /// A projection type that is parameterized by a list of projection column types.
    #[display("Projection({})", _0)]
    Projection(Projection),
}

impl Constructor {
    fn resolve(&self, unifier: &mut Unifier<'_>) -> Result<crate::Type, TypeError> {
        match self {
            Constructor::Value(value) => match value {
                Value::Eql(eql_col) => Ok(crate::Type::Value(crate::Value::Eql(eql_col.clone()))),
                Value::Native(native_col) => {
                    Ok(crate::Type::Value(crate::Value::Native(native_col.clone())))
                }
                Value::Array(element_tid) => {
                    let element_ty = unifier.lookup(*element_tid);
                    let resolved = element_ty.resolved(unifier)?;
                    Ok(crate::Type::Value(crate::Value::Array(resolved.into())))
                }
            },
            Constructor::Projection(projection) => {
                Ok(crate::Type::Projection(projection.resolve(unifier)?))
            }
        }
    }
}

impl Projection {
    fn resolve(&self, unifier: &mut Unifier<'_>) -> Result<crate::Projection, TypeError> {
        use itertools::Itertools;

        let resolved_cols =self
                    .flatten(unifier)
                    .columns()
                    .iter()
                    .map(|col| -> Result<Vec<crate::ProjectionColumn>, TypeError> {
                        let col_ty = unifier.lookup(col.tid);
                        let alias = col.alias.clone();
                        match col_ty {
                            Type::Constructor(constructor) => match constructor {
                                Constructor::Value(Value::Eql(eql_col)) => {
                                    Ok(vec![crate::ProjectionColumn {
                                        ty: crate::Value::Eql(eql_col),
                                        alias,
                                    }])
                                }
                                Constructor::Value(Value::Native(native_col)) => {
                                    Ok(vec![crate::ProjectionColumn {
                                        ty: crate::Value::Native(native_col),
                                        alias,
                                    }])
                                }
                                Constructor::Value(Value::Array(array_tid)) => {
                                    let array_ty = unifier.lookup(array_tid);
                                    match array_ty.resolved(unifier)? {
                                        elem_ty @ crate::Type::Value(_) => {
                                            Ok(vec![crate::ProjectionColumn {
                                                ty: crate::Value::Array(elem_ty.into()),
                                                alias,
                                            }])
                                        }
                                        crate::Type::Projection(_) => {
                                            Err(TypeError::InternalError(format!(
                                                "projection type as array element"
                                            )))
                                        }
                                    }
                                }
                                Constructor::Projection(_) => {
                                    Err(TypeError::InternalError(format!(
                                        "projection type as projection column; projections should be flattened during final resolution"
                                    )))
                                }
                            },
                            Type::Var(tvar) => {
                                let tid = unifier.lookup_substitution(tvar).ok_or(
                                    TypeError::InternalError(format!("could not resolve type variable '{}'", tvar)))?;
                                let ty = unifier.lookup(tid);
                                if let Type::Constructor(Constructor::Projection(projection)) = ty {
                                    match projection.resolve(unifier)? {
                                        crate::Projection::WithColumns(projection_columns) => Ok(projection_columns),
                                        crate::Projection::Empty => Ok(vec![]),
                                    }
                                } else {
                                    match ty.resolved(unifier)? {
                                        crate::Type::Value(value) => Ok(vec![crate::ProjectionColumn { ty: value, alias }]),
                                        crate::Type::Projection(_) => Err(TypeError::InternalError(format!("unexpected projection"))),
                                    }
                                }

                            },
                        }
                    })
                    .flatten_ok()
                    .collect::<Result<Vec<_>, _>>()?;

        if resolved_cols.len() == 0 {
            Ok(crate::Projection::Empty)
        } else {
            Ok(crate::Projection::WithColumns(resolved_cols))
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Display)]
pub enum Value {
    /// An encrypted type from a particular table-column in the schema.
    ///
    /// An encrypted column never shares a type with another encrypted column - which is why it is sufficient to
    /// identify the type by its table & column names.
    #[display("EQL({_0})")]
    Eql(EqlValue),

    /// A native database type that carries its table & column name.  `NativeValue(None)` & `NativeValue(Some(_))` are
    /// will successfully unify with each other - they are the same type as far as the type system is concerned.
    /// `NativeValue(Some(_))` just carries more information which makes testing & debugging easier.
    #[display("Native")]
    Native(NativeValue),

    /// An array type that is parameterized by an element type.
    #[display("Array({})", _0)]
    Array(TID),
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
#[display("{} {}", self.tid, self.render_alias())]
pub struct ProjectionColumn {
    /// The type of the column.
    pub tid: TID,

    /// The columm alias
    pub alias: Option<Ident>,
}

/// A placeholder for an unknown type.
#[derive(Debug, Copy, Clone, PartialEq, Eq, Hash, Display, Default)]
pub struct TypeVar(pub u32);

impl Type {
    /// Creates an `Type` containing an empty projection
    pub(crate) fn empty_projection() -> Type {
        Type::Constructor(Constructor::Projection(Projection::Empty))
    }

    /// Creates an `Type` containing a `Constructor::Scalar(Scalar::Native(NativeValue(None)))`.
    pub(crate) fn any_native() -> Type {
        Type::Constructor(Constructor::Value(Value::Native(NativeValue(None))))
    }

    /// Creates an `Type` containing a `Constructor::Projection`.
    pub(crate) fn projection(columns: &[(TID, Option<Ident>)]) -> Type {
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

    /// Creates an `Type` containing a `Constructor::Array`.
    pub(crate) fn array(element_ty: TID) -> Type {
        Type::Constructor(Constructor::Value(Value::Array(element_ty)))
    }

    /// Resolves `self`, returning it as a [`crate::Type`].
    ///
    /// A resolved type is one in which all type variables have been resolved, recursively.
    ///
    /// Fails with a [`TypeError`] if the stored `Type` cannot be fully resolved.
    pub fn resolved(&self, unifier: &mut Unifier<'_>) -> Result<crate::Type, TypeError> {
        match self {
            Type::Constructor(constructor) => constructor.resolve(unifier),
            Type::Var(type_var) => {
                if let Some(sub_tid) = unifier.lookup_substitution(*type_var) {
                    let sub_ty = unifier.lookup(sub_tid);
                    match sub_ty.resolved(unifier) {
                        Ok(sub_ty) => return Ok(sub_ty),
                        Err(err) => {
                            if unifier.exists_node_with_type::<ast::Value>(&Type::Var(*type_var)) {
                                let unified_tid = unifier.unify(sub_tid, TID::NATIVE)?;
                                return unifier.lookup(unified_tid).resolved(unifier);
                            } else {
                                return Err(err);
                            }
                        }
                    }
                }

                return Err(TypeError::Incomplete(format!(
                    "there are no substitutions for type var {}",
                    type_var
                )));
            }
        }
    }

    pub(crate) fn resolved_as<T: Clone + 'static>(
        &self,
        unifier: &mut Unifier<'_>,
    ) -> Result<T, TypeError> {
        let resolved_ty: crate::Type = self.resolved(unifier)?;

        let result = match &resolved_ty {
            crate::Type::Projection(projection) => {
                if let Some(t) = (projection as &dyn std::any::Any).downcast_ref::<T>() {
                    return Ok(t.clone());
                }

                Err(())
            }
            crate::Type::Value(value) => {
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

    pub(crate) fn flatten(&self, unifier: &Unifier<'_>) -> Self {
        match self {
            Projection::WithColumns(projection_columns) => {
                Projection::WithColumns(projection_columns.flatten(unifier))
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

    pub(crate) fn flatten(&self, unifier: &Unifier<'_>) -> Self {
        ProjectionColumns(self.flatten_impl(unifier, Vec::with_capacity(self.len())))
    }

    fn flatten_impl(
        &self,
        unifier: &Unifier<'_>,
        mut output: Vec<ProjectionColumn>,
    ) -> Vec<ProjectionColumn> {
        for ProjectionColumn { tid, alias } in &self.0 {
            match unifier.lookup(*tid) {
                Type::Constructor(Constructor::Projection(Projection::WithColumns(nested))) => {
                    output = nested.flatten_impl(unifier, output);
                }
                _ => output.push(ProjectionColumn::new(*tid, alias.clone())),
            }
        }
        output
    }
}

impl ProjectionColumn {
    /// Returns a new `ProjectionColumn` with type `ty` and optional `alias`.
    pub(crate) fn new(ty: TID, alias: Option<Ident>) -> Self {
        Self { tid: ty, alias }
    }

    fn render_alias(&self) -> String {
        match &self.alias {
            Some(name) => name.to_string(),
            None => String::from("(no-alias)"),
        }
    }
}

impl ProjectionColumns {
    pub(crate) fn new_from_schema_table(
        table: Arc<Table>,
        registry: &mut TypeRegistry<'_>,
    ) -> Self {
        ProjectionColumns(
            table
                .columns
                .iter()
                .map(|col| {
                    let tc = TableColumn {
                        table: table.name.clone(),
                        column: col.name.clone(),
                    };

                    let value_tid = if col.kind == ColumnKind::Native {
                        registry.register(Type::Constructor(Constructor::Value(Value::Native(
                            NativeValue(Some(tc)),
                        ))))
                    } else {
                        registry.register(Type::Constructor(Constructor::Value(Value::Eql(
                            EqlValue(tc),
                        ))))
                    };

                    ProjectionColumn::new(value_tid, Some(col.name.clone()))
                })
                .collect(),
        )
    }
}
