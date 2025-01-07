use std::{cell::RefCell, rc::Rc};

mod types;

use crate::inference::TypeError;

pub(crate) use types::*;

pub use types::{EqlValue, NativeValue, TableColumn};

use super::TypeRegistry;
use tracing::{span, Level};

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

        let span = span!(
            Level::DEBUG,
            "unify",
            depth = self.depth,
            left = &*left.to_string(),
            right = &*right.to_string()
        );

        let _guard = span.enter();

        self.depth += 1;

        // If left & right are equal we can short circuit unification.
        if left == right {
            return Ok(left.clone());
        }

        let unification = match (left, right) {
            // Two projections unify if they have the same number of columns and all of the paired column types also
            // unify.
            (
                Type::Constructor(Projection(lhs_projection)),
                Type::Constructor(Projection(rhs_projection)),
            ) => Ok(Type::Constructor(Projection(
                self.unify_projections(lhs_projection, rhs_projection)?,
            ))),

            // Two arrays unify if the types of their element types unify.
            (
                Type::Constructor(Value(Array(element_ty_left))),
                Type::Constructor(Value(Array(element_ty_right))),
            ) => {
                let element_ty = self.unify(element_ty_left, element_ty_right)?;

                Ok(Type::Constructor(Value(Array(element_ty.into()))))
            }

            // A Value can unify with a single column projection
            (Type::Constructor(Value(value)), Type::Constructor(Projection(projection)))
            | (Type::Constructor(Projection(projection)), Type::Constructor(Value(value))) => {
                let len = projection.len();
                if len == 1 {
                    Ok(self.unify_value_with_type(value, &projection[0].ty)?)
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
        left: &Projection,
        right: &Projection,
    ) -> Result<Projection, TypeError> {
        let left = left.flatten();
        let right = right.flatten();

        if left.len() == right.len() {
            let unified: Vec<ProjectionColumn> = left
                .columns()
                .iter()
                .zip(right.columns().iter())
                .map(|(lhs, rhs)| {
                    self.unify(&lhs.ty, &rhs.ty).map(|ty| ProjectionColumn {
                        ty,
                        // Unification of projections occurs in set operations such as UNION.  The aliases of the
                        // columns in the left hand side argument to the set operator *always* win - even when the
                        // corresponding right hand side column has an alias and the left hand side does not.
                        alias: lhs.alias.clone(),
                    })
                })
                .collect::<Result<Vec<_>, _>>()?;

            Ok(Projection::new(unified))
        } else {
            Err(TypeError::Conflict(format!(
                "cannot unify projections {} and {} because they have different numbers of columns",
                left, right
            )))
        }
    }

    fn unify_value_with_type(&mut self, value: &Value, ty: &Type) -> Result<Type, TypeError> {
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

    #[ignore = "this is addressed in unmerged PR"]
    #[test]
    fn eq_never() {
        let left = Type::Constructor(Projection(crate::unifier::Projection::Empty));
        let right = Type::Constructor(Projection(crate::unifier::Projection::Empty));

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        assert_eq!(unifier.unify(&left, &right), Ok(left.clone()));
    }

    #[test]
    fn constructor_with_var() {
        let left = Type::Constructor(Value(Native(NativeValue(None))));
        let right = Type::Var(TypeVar(0));

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));
        let unified = unifier.unify(&left, &right);

        assert_eq!(unified, Ok(left));
    }

    #[test]
    fn var_with_constructor() {
        let left = Type::Var(TypeVar(0));
        let right = Type::Constructor(Value(Native(NativeValue(None))));

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));
        let unified = unifier.unify(&left, &right);

        assert_eq!(unified, Ok(right));
    }

    #[test]
    fn projections_without_wildcards() {
        let left = Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
            ProjectionColumns(vec![
                ProjectionColumn {
                    ty: Type::Constructor(Value(Native(NativeValue(None)))),
                    alias: None,
                },
                ProjectionColumn {
                    ty: Type::Var(TypeVar(0)),
                    alias: None,
                },
            ]),
        )));

        let right = Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
            ProjectionColumns(vec![
                ProjectionColumn {
                    ty: Type::Var(TypeVar(1)),
                    alias: None,
                },
                ProjectionColumn {
                    ty: Type::Constructor(Value(Native(NativeValue(None)))),
                    alias: None,
                },
            ]),
        )));

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));
        let unified = unifier.unify(&left, &right).unwrap();

        assert_eq!(
            unified,
            Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
                ProjectionColumns(vec![
                    ProjectionColumn {
                        ty: Type::Constructor(Value(Native(NativeValue(None)))),
                        alias: None
                    },
                    ProjectionColumn {
                        ty: Type::Constructor(Value(Native(NativeValue(None)))),
                        alias: None
                    },
                ])
            )))
        );
    }

    #[test]
    fn projections_with_wildcards() {
        let left = Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
            ProjectionColumns(vec![
                ProjectionColumn {
                    ty: Type::Constructor(Value(Native(NativeValue(None)))),
                    alias: None,
                },
                ProjectionColumn {
                    ty: Type::Constructor(Value(Native(NativeValue(None)))),
                    alias: None,
                },
            ]),
        )));

        // The RHS is a single projection that contains a projection column that contains a projection with two
        // projection columns.  This is how wildcard expansions is represented at the type level.
        let right = Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
            ProjectionColumns(vec![ProjectionColumn {
                ty: Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
                    ProjectionColumns(vec![
                        ProjectionColumn {
                            ty: Type::Constructor(Value(Native(NativeValue(None)))),
                            alias: None,
                        },
                        ProjectionColumn {
                            ty: Type::Constructor(Value(Native(NativeValue(None)))),
                            alias: None,
                        },
                    ]),
                ))),
                alias: None,
            }]),
        )));

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));
        let unified = unifier.unify(&left, &right).unwrap();

        assert_eq!(
            unified,
            Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
                ProjectionColumns(vec![
                    ProjectionColumn {
                        ty: Type::Constructor(Value(Native(NativeValue(None)))),
                        alias: None
                    },
                    ProjectionColumn {
                        ty: Type::Constructor(Value(Native(NativeValue(None)))),
                        alias: None
                    },
                ])
            )))
        );
    }
}
