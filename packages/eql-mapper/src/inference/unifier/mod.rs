use std::{cell::RefCell, cmp::max, rc::Rc};

mod types;

use crate::inference::TypeError;

use sqlparser::ast::Ident;
pub(crate) use types::*;

pub use types::{EqlValue, NativeValue, TableColumn};

use super::TypeRegistry;
use tracing::{info, span, Level};

/// Implements the type unification algorithm and maintains an association of type variables with the type that they
/// point to.
#[derive(Debug)]
pub struct Unifier<'ast> {
    registry: Rc<RefCell<TypeRegistry<'ast>>>,
    depth: usize,
}

impl<'ast> Unifier<'ast> {
    /// Creates a new `Unifier`.
    pub fn new(reg: impl Into<Rc<RefCell<TypeRegistry<'ast>>>>) -> Self {
        Self {
            registry: reg.into(),
            depth: 0,
        }
    }

    /// Unifies two [`Type`]s or fails with a [`TypeError`].
    ///
    /// "Type Unification" is a fancy term for finding a set of type variable substitutions for multiple types
    /// that make those types equal, or else fail with a type error.
    ///
    /// Successful unification does not guarantee that the returned type will be fully resolved (i.e. it can contain
    /// dangling type variables).
    ///
    /// Returns `Ok(ty)` if successful, or `Err(TypeError)` on failure.
    pub(crate) fn unify(&mut self, left: &Type, right: &Type) -> Result<Type, TypeError> {
        use types::Constructor::*;
        use types::Value::*;

        // If left & right are equal we can short circuit unification.
        if left == right {
            return Ok(left.clone());
        }

        let span = span!(
            Level::DEBUG,
            "unify",
            depth = self.depth,
            left = &*left.to_string(),
            right = &*right.to_string()
        );

        let _guard = span.enter();

        self.depth += 1;

        info!(
            "{:indent$}  {} UNIFY {}",
            " ",
            left,
            right,
            indent = (self.depth - 1) * 4
        );

        let unification = match (left, right) {
            // Two projections unify if they have the same number of columns and all of the paired column types also
            // unify.
            (
                Type::Constructor(Projection(cols_left)),
                Type::Constructor(Projection(cols_right)),
            ) => Ok(Type::Constructor(Projection(ProjectionColumns(
                self.unify_projections(&cols_left.0[..], &cols_right.0[..])?,
            )))),

            // Two arrays unify if the types of their element types unify.
            (
                Type::Constructor(Value(Array(element_ty_left))),
                Type::Constructor(Value(Array(element_ty_right))),
            ) => {
                let element_ty = self.unify(element_ty_left, element_ty_right)?;

                Ok(Type::Constructor(Value(Array(element_ty.into()))))
            }

            // A Value can unify with a single column projection
            (Type::Constructor(Value(value)), Type::Constructor(Projection(columns)))
            | (Type::Constructor(Projection(columns)), Type::Constructor(Value(value))) => {
                let len = columns.0.len();
                if len == 1 {
                    Ok(self.unify_value_with_single_column_projection(value, &columns.0[0].ty)?)
                } else {
                    Err(TypeError::Conflict(
                        "cannot unify value type with projection of more than one column"
                            .to_string(),
                    ))
                }
            }

            (Type::Constructor(Value(Native(_))), Type::Constructor(Value(Native(_)))) => {
                Ok(left.clone())
            }

            (Type::Constructor(Value(Eql(lhs))), Type::Constructor(Value(Eql(rhs)))) => {
                if lhs == rhs {
                    Ok(left.clone())
                } else {
                    Err(TypeError::Conflict(format!(
                        "cannot unify different EQL types: {} and {}",
                        lhs, rhs
                    )))
                }
            }

            // A constructor resolves with a type variable if either:
            // 1. the type variable does not already refer to a constructor (transitively), or
            // 2. it does refer to a constructor and the two constructors unify
            (ty, Type::Var(tvar)) | (Type::Var(tvar), ty) => {
                Ok(self.unify_with_type_var(ty, *tvar)?)
            }

            // Any other combination of types is a type error.
            (lhs, rhs) => Err(TypeError::Conflict(format!(
                "type {} cannot be unified with {}",
                lhs, rhs
            ))),
        };

        let unification = match unification {
            Ok(Type::Constructor(Constructor::Projection(cols))) => {
                Ok(Type::Constructor(Constructor::Projection(cols.flatten())))
            }
            other => other,
        };

        if let Ok(unification) = &unification {
            info!(
                "= {:indent$} {}",
                "",
                unification,
                indent = (self.depth - 1) * 4
            );
        }

        self.depth -= 1;

        unification
    }

    /// Unifies a type with a type variable.
    ///
    /// Attempts to unify the type with whatever the type variable is pointing to.
    ///
    /// After successful unification `ty_rc` and `tvar_rc` will refer to the same allocation.
    fn unify_with_type_var(&mut self, ty: &Type, tvar: TypeVar) -> Result<Type, TypeError> {
        let sub = {
            let reg = &*self.registry.borrow();
            reg.get_substitution(tvar)
        };

        let ty = match sub {
            Some((sub_ty, _)) => self.unify(ty, &sub_ty)?,
            None => ty.clone(),
        };

        self.registry.borrow_mut().substitute(tvar, ty.clone());

        Ok(ty)
    }

    /// Unifies two slices of [`ProjectionColumn`]s.
    ///
    /// Wildcard selections are represented in the type system as nested projections. That means `left` & `right` could
    /// have different numbers of `ProjectionColumn`s yet could still successfully unify.
    fn unify_projections(
        &mut self,
        left: &[ProjectionColumn],
        right: &[ProjectionColumn],
    ) -> Result<Vec<ProjectionColumn>, TypeError> {
        let output: Vec<ProjectionColumn> = Vec::with_capacity(max(left.len(), right.len()));
        self.unify_projections_recursive(left, right, output)
    }

    /// Unifies two projections, storing the unified version in `output`.
    ///
    /// The algorithm works by unifying the first columns of left and right and the proceeding to recursively unifying
    /// the rest of each slice (again, from the front).
    ///
    /// The trickiness occurs when one of the columns itself contains a projection (because wildcards).
    fn unify_projections_recursive(
        &mut self,
        left: &[ProjectionColumn],
        right: &[ProjectionColumn],
        mut output: Vec<ProjectionColumn>,
    ) -> Result<Vec<ProjectionColumn>, TypeError> {
        match (&left.first(), &right.first()) {
            // Nothing left to do
            (None, None) => Ok(output),

            // One side has nothing left to match but the other side has unmatched columns.
            // This is an error.
            (None, Some(unmatched)) | (Some(unmatched), None) => Err(TypeError::Conflict(format!(
                "projections {:?} and {:?} could not be unified due to unmatched columns: {:?}",
                left, right, unmatched
            ))),

            (
                Some(ProjectionColumn {
                    ty: Type::Constructor(left_ctor),
                    alias: left_alias,
                }),
                Some(ProjectionColumn {
                    ty: Type::Constructor(right_ctor),
                    alias: right_alias,
                }),
            ) => match (
                left_ctor,
                right_ctor,
                self.unify_alias(left_alias, right_alias)?,
            ) {
                (
                    Constructor::Value(Value::Array(lhs)),
                    Constructor::Value(Value::Array(rhs)),
                    alias,
                ) => {
                    let unified = self.unify(lhs, rhs)?;
                    output.push(ProjectionColumn { ty: unified, alias });
                    self.unify_projections_recursive(&left[1..], &right[1..], output)
                }

                (Constructor::Value(lhs), Constructor::Value(rhs), alias) if lhs == rhs => {
                    output.push(ProjectionColumn {
                        ty: Type::Constructor(left_ctor.clone()),
                        alias,
                    });
                    self.unify_projections_recursive(&left[1..], &right[1..], output)
                }

                (Constructor::Projection(left_cols), Constructor::Projection(right_cols), _) => {
                    output = self.unify_projections_recursive(
                        &left_cols.0[..],
                        &right_cols.0[..],
                        output,
                    )?;
                    self.unify_projections_recursive(&left[1..], &right[1..], output)
                }

                (Constructor::Empty, Constructor::Empty, _) => {
                    self.unify_projections_recursive(&left[1..], &right[1..], output)
                }

                (Constructor::Value(_), Constructor::Projection(right_proj), _) => {
                    output = self.unify_projections_recursive(left, &right_proj.0[..], output)?;
                    self.unify_projections_recursive(&left[1..], &right[1..], output)
                }

                (Constructor::Projection(left_proj), Constructor::Value(_), _) => {
                    output = self.unify_projections_recursive(&left_proj.0[..], right, output)?;
                    self.unify_projections_recursive(&left[1..], &right[1..], output)
                }

                (lhs, rhs, _) => Err(TypeError::Conflict(format!(
                    "types {:?} and {:?} could not be unified",
                    lhs, rhs
                ))),
            },

            (
                Some(ProjectionColumn {
                    ty: Type::Var(tvar),
                    alias: left_alias,
                }),
                Some(ProjectionColumn {
                    ty,
                    alias: right_alias,
                }),
            ) => {
                let unified = self.unify_with_type_var(ty, *tvar)?;
                output.push(ProjectionColumn {
                    ty: unified,
                    alias: self.unify_alias(left_alias, right_alias)?,
                });
                self.unify_projections_recursive(&left[1..], &right[1..], output)
            }

            (
                Some(ProjectionColumn {
                    ty,
                    alias: left_alias,
                }),
                Some(ProjectionColumn {
                    ty: Type::Var(tvar),
                    alias: right_alias,
                }),
            ) => {
                let unified = self.unify_with_type_var(ty, *tvar)?;
                output.push(ProjectionColumn {
                    ty: unified,
                    alias: self.unify_alias(left_alias, right_alias)?,
                });
                self.unify_projections_recursive(&left[1..], &right[1..], output)
            }
        }
    }

    fn unify_alias(
        &self,
        left: &Option<Ident>,
        right: &Option<Ident>,
    ) -> Result<Option<Ident>, TypeError> {
        match (left, right) {
            (None, None) => Ok(None),
            (None, Some(alias)) => Ok(Some(alias.clone())),
            (Some(alias), None) => Ok(Some(alias.clone())),
            (Some(a), Some(b)) if a == b => Ok(Some(a.clone())),
            (Some(a), Some(b)) => Err(TypeError::Conflict(format!(
                "projection column aliases are not equal: {} and {}",
                a, b
            ))),
        }
    }

    pub(crate) fn render_projection(columns: &ProjectionColumns) -> String {
        let ty_strings: Vec<String> = columns.0.iter().map(|col| col.ty.to_string()).collect();
        format!("[{}]", ty_strings.join(", "))
    }

    fn unify_value_with_single_column_projection(
        &mut self,
        value: &Value,
        ty: &Type,
    ) -> Result<Type, TypeError> {
        match (value, ty) {
            (Value::Eql(left), Type::Constructor(Constructor::Value(Value::Eql(right))))
                if left == right =>
            {
                Ok(ty.clone())
            }
            (Value::Native(_), Type::Constructor(Constructor::Value(Value::Native(_)))) => {
                Ok(ty.clone())
            }
            (Value::Array(left), Type::Constructor(Constructor::Value(Value::Array(right)))) => {
                self.unify(left, right)
            }
            (Value::Eql(left), tvar @ Type::Var(_)) => self.unify(
                &Type::Constructor(Constructor::Value(Value::Eql(left.clone()))),
                tvar,
            ),
            (Value::Native(left), tvar @ Type::Var(_)) => self.unify(
                &Type::Constructor(Constructor::Value(Value::Native(left.clone()))),
                tvar,
            ),
            (Value::Array(left), tvar @ Type::Var(_)) => self.unify(
                &Type::Constructor(Constructor::Value(Value::Array(left.clone()))),
                tvar,
            ),
            _ => Err(TypeError::Conflict(format!(
                "value type {} cannot be unified with single column projection of {}",
                value, ty
            ))),
        }
    }
}

#[cfg(test)]
mod test {
    use crate::unifier::{Constructor::*, NativeValue, ProjectionColumn, Type, TypeVar, Value::*};
    use crate::unifier::{ProjectionColumns, Unifier};
    use crate::{DepMut, TypeRegistry};

    #[test]
    fn eq_native() {
        let left = Type::Constructor(Value(Native(NativeValue(None))));
        let right = Type::Constructor(Value(Native(NativeValue(None))));

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        assert_eq!(unifier.unify(&left, &right), Ok(left.clone()));
    }

    #[test]
    fn eq_never() {
        let left = Type::Constructor(Empty);
        let right = Type::Constructor(Empty);

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        assert_eq!(unifier.unify(&left, &right), Ok(left.clone()));
    }

    #[test]
    fn constructor_with_var() {
        let left = Type::Constructor(Value(Native(NativeValue(None))));
        let right = Type::Var(TypeVar(0));

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));
        let unified = unifier.unify(&left, &right).unwrap();

        assert_eq!(unified, left);
    }

    #[test]
    fn var_with_constructor() {
        let left = Type::Var(TypeVar(0));
        let right = Type::Constructor(Value(Native(NativeValue(None))));

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));
        let unified = unifier.unify(&left, &right).unwrap();

        assert_eq!(unified, left);
    }

    #[test]
    fn projections_without_wildcards() {
        let left = Type::Constructor(Projection(ProjectionColumns(vec![
            ProjectionColumn {
                ty: Type::Constructor(Value(Native(NativeValue(None)))),
                alias: None,
            },
            ProjectionColumn {
                ty: Type::Var(TypeVar(0)),
                alias: None,
            },
        ])));

        let right = Type::Constructor(Projection(ProjectionColumns(vec![
            ProjectionColumn {
                ty: Type::Var(TypeVar(1)),
                alias: None,
            },
            ProjectionColumn {
                ty: Type::Constructor(Empty),
                alias: None,
            },
        ])));

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));
        let unified = unifier.unify(&left, &right).unwrap();

        assert_eq!(
            unified,
            Type::Constructor(Projection(ProjectionColumns(vec![
                ProjectionColumn {
                    ty: Type::Constructor(Value(Native(NativeValue(None)))),
                    alias: None
                },
                ProjectionColumn {
                    ty: Type::Constructor(Empty),
                    alias: None
                },
            ])))
        );

        assert_eq!(right, left);
    }

    #[test]
    fn projections_with_wildcards() {
        let left = Type::Constructor(Projection(ProjectionColumns(vec![
            ProjectionColumn {
                ty: Type::Constructor(Value(Native(NativeValue(None)))),
                alias: None,
            },
            ProjectionColumn {
                ty: Type::Constructor(Empty),
                alias: None,
            },
        ])));

        // The RHS is a single projection that contains a projection column that contains a projection with two
        // projection columns.  This is how wildcard expansions is represented at the type level.
        let right = Type::Constructor(Projection(ProjectionColumns(vec![ProjectionColumn {
            ty: Type::Constructor(Projection(ProjectionColumns(vec![
                ProjectionColumn {
                    ty: Type::Constructor(Value(Native(NativeValue(None)))),
                    alias: None,
                },
                ProjectionColumn {
                    ty: Type::Constructor(Empty),
                    alias: None,
                },
            ]))),
            alias: None,
        }])));

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));
        let unified = unifier.unify(&left, &right).unwrap();

        assert_eq!(
            unified,
            Type::Constructor(Projection(ProjectionColumns(vec![
                ProjectionColumn {
                    ty: Type::Constructor(Value(Native(NativeValue(None)))),
                    alias: None
                },
                ProjectionColumn {
                    ty: Type::Constructor(Empty),
                    alias: None
                },
            ])))
        );

        assert_eq!(right, left);
    }
}
