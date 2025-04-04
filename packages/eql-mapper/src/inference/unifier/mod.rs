use std::{cell::RefCell, rc::Rc};

mod type_cell;
mod types;

use crate::inference::TypeError;

pub(crate) use type_cell::*;
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
    pub(crate) fn unify(&mut self, left: TypeCell, right: TypeCell) -> Result<TypeCell, TypeError> {
        use types::Constructor::*;
        use types::Value::*;

        let span = span!(
            Level::DEBUG,
            "unify",
            depth = self.depth,
            left = left.as_type().to_string(),
            right = right.as_type().to_string()
        );

        let _guard = span.enter();

        self.depth += 1;

        // Short-circuit the unification when left & right are equal.
        if left == right {
            return Ok(left.join(&right));
        }

        let (a, b) = (left.as_type(), right.as_type());

        let unification = match (&*a, &*b) {
            // Two projections unify if they have the same number of columns and all of the paired column types also
            // unify.
            (Type::Constructor(Projection(_)), Type::Constructor(Projection(_))) => {
                Ok(self.unify_projections(left.clone(), right.clone())?)
            }

            // Two arrays unify if the types of their element types unify.
            (
                Type::Constructor(Value(Array(element_ty_left))),
                Type::Constructor(Value(Array(element_ty_right))),
            ) => {
                let element_ty = self.unify(element_ty_left.clone(), element_ty_right.clone())?;

                Ok(left.join_all(&[
                    &right,
                    &TypeCell::new(Type::Constructor(Value(Array(element_ty)))),
                ]))
            }

            // A Value can unify with a single column projection
            (Type::Constructor(Value(_)), Type::Constructor(Projection(projection))) => {
                let projection = projection.flatten();
                let len = projection.len();
                if len == 1 {
                    let unified =
                        self.unify_value_type_with_type(left.clone(), projection[0].ty.clone())?;

                    Ok(TypeCell::join_all(&left, &[&right, &unified]))
                } else {
                    Err(TypeError::Conflict(
                        "cannot unify value type with projection of more than one column"
                            .to_string(),
                    ))
                }
            }

            (Type::Constructor(Projection(projection)), Type::Constructor(Value(_))) => {
                let projection = projection.flatten();
                let len = projection.len();
                if len == 1 {
                    let unified =
                        self.unify_value_type_with_type(right.clone(), projection[0].ty.clone())?;
                    Ok(TypeCell::join_all(&left, &[&right, &unified]))
                } else {
                    Err(TypeError::Conflict(
                        "cannot unify value type with projection of more than one column"
                            .to_string(),
                    ))
                }
            }

            (Type::Constructor(Value(Native(_))), Type::Constructor(Value(Native(_)))) => {
                Ok(left.join(&right))
            }

            (Type::Constructor(Value(Eql(lhs))), Type::Constructor(Value(Eql(rhs)))) => {
                if lhs == rhs {
                    Ok(left.join(&right))
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
            (_, Type::Var(tvar)) => {
                let unified = self.unify_with_type_var(left.clone(), *tvar)?;
                Ok(TypeCell::join_all(&left, &[&right, &unified]))
            }

            // A constructor resolves with a type variable if either:
            // 1. the type variable does not already refer to a constructor (transitively), or
            // 2. it does refer to a constructor and the two constructors unify
            (Type::Var(tvar), _) => {
                let unified = self.unify_with_type_var(right.clone(), *tvar)?;
                Ok(TypeCell::join_all(&left, &[&right, &unified]))
            }

            // Any other combination of types is a type error.
            (lhs, rhs) => Err(TypeError::Conflict(format!(
                "type {} cannot be unified with {}",
                lhs, rhs
            ))),
        };

        self.depth -= 1;

        unification
    }

    /// Unifies a type with a type variable.
    ///
    /// Attempts to unify the type with whatever the type variable is pointing to.
    ///
    /// After successful unification `ty_rc` and `tvar_rc` will refer to the same allocation.
    fn unify_with_type_var(&mut self, ty: TypeCell, tvar: TypeVar) -> Result<TypeCell, TypeError> {
        let sub = {
            let reg = &*self.registry.borrow();
            reg.get_substitution(tvar)
        };

        let ty = match sub {
            Some(sub_ty) => self.unify(ty.clone(), sub_ty.clone())?,
            None => ty.clone(),
        };

        self.registry.borrow_mut().substitute(tvar, ty.clone());

        Ok(ty)
    }

    /// Unifies two projection types.
    fn unify_projections(
        &mut self,
        left: TypeCell,
        right: TypeCell,
    ) -> Result<TypeCell, TypeError> {
        match (&*left.as_type(), &*right.as_type()) {
            (
                Type::Constructor(Constructor::Projection(lhs_projection)),
                Type::Constructor(Constructor::Projection(rhs_projection)),
            ) => {
                let lhs_projection = lhs_projection.flatten();
                let rhs_projection = rhs_projection.flatten();

                if lhs_projection.len() == rhs_projection.len() {
                    let mut cols: Vec<ProjectionColumn> = Vec::with_capacity(lhs_projection.len());

                    for (lhs_col, rhs_col) in lhs_projection
                        .columns()
                        .iter()
                        .zip(rhs_projection.columns())
                    {
                        cols.push(ProjectionColumn::new(
                            self.unify(lhs_col.ty.clone(), rhs_col.ty.clone())?,
                            lhs_col.alias.clone(),
                        ));
                    }
                    let unified = TypeCell::new(Type::Constructor(Constructor::Projection(
                        Projection::new(cols),
                    )));

                    Ok(left.join_all(&[&right, &unified]))
                } else {
                    Err(TypeError::Conflict(format!(
                        "cannot unify projections {} and {} because they have different numbers of columns",
                        *left.as_type(), *right.as_type()
                    )))
                }
            }
            (_, _) => Err(TypeError::InternalError(
                "unify_projections expected projection types".to_string(),
            )),
        }
    }

    fn unify_value_type_with_type(
        &mut self,
        value: TypeCell,
        ty: TypeCell,
    ) -> Result<TypeCell, TypeError> {
        let (a, b) = (value.as_type(), ty.as_type());

        match (&*a, &*b) {
            (
                Type::Constructor(Constructor::Value(Value::Eql(left))),
                Type::Constructor(Constructor::Value(Value::Eql(right))),
            ) if left == right => Ok(value.join(&ty)),
            (
                Type::Constructor(Constructor::Value(Value::Native(_))),
                Type::Constructor(Constructor::Value(Value::Native(_))),
            ) => Ok(value.join(&ty)),
            (
                Type::Constructor(Constructor::Value(Value::Array(left))),
                Type::Constructor(Constructor::Value(Value::Array(right))),
            ) => {
                self.unify(left.clone(), right.clone())?;
                Ok(value.join(&ty))
            }
            (Type::Constructor(Constructor::Value(Value::Eql(_))), Type::Var(tvar)) => {
                let unified = self.unify_with_type_var(value.clone(), *tvar)?;
                Ok(value.join_all(&[&ty, &unified]))
            }
            (Type::Var(tvar), Type::Constructor(Constructor::Value(Value::Eql(_)))) => {
                let unified = self.unify_with_type_var(value.clone(), *tvar)?;
                Ok(value.join_all(&[&ty, &unified]))
            }
            _ => Err(TypeError::Conflict(format!(
                "value type {} cannot be unified with single column projection of {}",
                *value.as_type(),
                *ty.as_type()
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
        let left = Type::Constructor(Value(Native(NativeValue(None)))).into_type_cell();
        let right = Type::Constructor(Value(Native(NativeValue(None)))).into_type_cell();

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        assert_eq!(unifier.unify(left.clone(), right), Ok(left));
    }

    #[ignore = "this is addressed in unmerged PR"]
    #[test]
    fn eq_never() {
        let left =
            Type::Constructor(Projection(crate::unifier::Projection::Empty)).into_type_cell();
        let right =
            Type::Constructor(Projection(crate::unifier::Projection::Empty)).into_type_cell();

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        assert_eq!(unifier.unify(left.clone(), right.clone()), Ok(left));
    }

    #[test]
    fn constructor_with_var() {
        let left = Type::Constructor(Value(Native(NativeValue(None)))).into_type_cell();
        let right = Type::Var(TypeVar(0)).into_type_cell();

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));
        let unified = unifier.unify(left.clone(), right.clone());

        assert_eq!(unified, Ok(left));
    }

    #[test]
    fn var_with_constructor() {
        let left = Type::Var(TypeVar(0)).into_type_cell();
        let right = Type::Constructor(Value(Native(NativeValue(None)))).into_type_cell();

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));
        let unified = unifier.unify(left.clone(), right.clone());

        assert_eq!(unified, Ok(right));
    }

    #[test]
    fn projections_without_wildcards() {
        let left = Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
            ProjectionColumns(vec![
                ProjectionColumn::new(
                    Type::Constructor(Value(Native(NativeValue(None)))).into_type_cell(),
                    None,
                ),
                ProjectionColumn::new(Type::Var(TypeVar(0)).into_type_cell(), None),
            ]),
        )))
        .into_type_cell();

        let right = Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
            ProjectionColumns(vec![
                ProjectionColumn::new(Type::Var(TypeVar(1)).into_type_cell(), None),
                ProjectionColumn::new(
                    Type::Constructor(Value(Native(NativeValue(None)))).into_type_cell(),
                    None,
                ),
            ]),
        )))
        .into_type_cell();

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));
        let unified = unifier.unify(left.clone(), right.clone()).unwrap();

        assert_eq!(
            unified,
            Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
                ProjectionColumns(vec![
                    ProjectionColumn::new(
                        Type::Constructor(Value(Native(NativeValue(None)))).into_type_cell(),
                        None
                    ),
                    ProjectionColumn::new(
                        Type::Constructor(Value(Native(NativeValue(None)))).into_type_cell(),
                        None
                    ),
                ])
            )))
            .into_type_cell()
        );
    }

    #[test]
    fn projections_with_wildcards() {
        let left = Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
            ProjectionColumns(vec![
                ProjectionColumn::new(
                    Type::Constructor(Value(Native(NativeValue(None)))).into_type_cell(),
                    None,
                ),
                ProjectionColumn::new(
                    Type::Constructor(Value(Native(NativeValue(None)))).into_type_cell(),
                    None,
                ),
            ]),
        )))
        .into_type_cell();

        // The RHS is a single projection that contains a projection column that contains a projection with two
        // projection columns.  This is how wildcard expansions is represented at the type level.
        let right = Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
            ProjectionColumns(vec![ProjectionColumn::new(
                Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
                    ProjectionColumns(vec![
                        ProjectionColumn::new(
                            Type::Constructor(Value(Native(NativeValue(None)))).into_type_cell(),
                            None,
                        ),
                        ProjectionColumn::new(
                            Type::Constructor(Value(Native(NativeValue(None)))).into_type_cell(),
                            None,
                        ),
                    ]),
                )))
                .into_type_cell(),
                None,
            )]),
        )))
        .into_type_cell();

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));
        let unified = unifier.unify(left.clone(), right.clone()).unwrap();

        assert_eq!(
            unified,
            Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
                ProjectionColumns(vec![
                    ProjectionColumn::new(
                        Type::Constructor(Value(Native(NativeValue(None)))).into_type_cell(),
                        None
                    ),
                    ProjectionColumn::new(
                        Type::Constructor(Value(Native(NativeValue(None)))).into_type_cell(),
                        None
                    ),
                ])
            )))
            .into_type_cell()
        );
    }
}
