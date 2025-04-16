use std::{cell::RefCell, rc::Rc, sync::Arc};

mod types;

use crate::inference::TypeError;

use sqltk::Visitable;
pub(crate) use types::*;

pub use types::{EqlValue, NativeValue, TableColumn};

use super::TypeRegistry;
use tracing::{span, Level};

/// Implements the type unification algorithm and maintains an association of type variables with the type that they
/// point to.
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

    /// Looks up a previously registered [`Type`] by its [`TypeVar`].
    pub(crate) fn lookup(&self, tvar: TypeVar) -> Option<Arc<Type>> {
        self.registry.borrow().get_type_by_tvar(tvar)
    }

    pub(crate) fn lookup_type_by_node<N: Visitable>(&self, node: &'ast N) -> Arc<Type> {
        self.registry.borrow().get_node_type(node)
    }

    pub(crate) fn exists_node_with_type<N: Visitable>(&self, ty: &Type) -> bool {
        self.registry.borrow().exists_node_with_type::<N>(ty)
    }

    pub(crate) fn substitute(&mut self, tvar: TypeVar, sub_ty: impl Into<Arc<Type>>) -> Arc<Type> {
        self.registry.borrow_mut().substitute(tvar, sub_ty)
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
    pub(crate) fn unify(
        &mut self,
        lhs: impl Into<Arc<Type>>,
        rhs: impl Into<Arc<Type>>,
    ) -> Result<Arc<Type>, TypeError> {
        use types::Constructor::*;
        use types::Value::*;

        let lhs: Arc<Type> = lhs.into();
        let rhs: Arc<Type> = rhs.into();

        let span_begin = span!(
            Level::DEBUG,
            "unify",
            depth = self.depth,
            lhs = ?lhs,
            rhs = ?rhs,
        );

        let _guard = span_begin.enter();

        self.depth += 1;

        // Short-circuit the unification when lhs & rhs are equal.
        if lhs == rhs {
            return Ok(lhs.clone());
        }

        let unification = match (&*lhs, &*rhs) {
            // Two projections unify if they have the same number of columns and all of the paired column types also
            // unify.
            (Type::Constructor(Projection(_)), Type::Constructor(Projection(_))) => {
                self.unify_projections(lhs, rhs)
            }

            // Two arrays unify if the types of their element types unify.
            (
                Type::Constructor(Value(Array(lhs_element_ty))),
                Type::Constructor(Value(Array(rhs_element_ty))),
            ) => {
                let unified_element_ty = self.unify(lhs_element_ty.clone(), rhs_element_ty.clone())?;
                let unified_array_ty = Type::Constructor(Value(Array(unified_element_ty)));
                Ok(unified_array_ty.into())
            }

            // A Value can unify with a single column projection
            (Type::Constructor(Value(_)), Type::Constructor(Projection(projection))) => {
                let projection = projection.flatten(self);
                let len = projection.len();
                if len == 1 {
                    self.unify_value_type_with_one_col_projection(lhs, projection[0].ty.clone())
                } else {
                    Err(TypeError::Conflict(
                        "cannot unify value type with projection of more than one column"
                            .to_string(),
                    ))
                }
            }

            (Type::Constructor(Projection(projection)), Type::Constructor(Value(_))) => {
                let projection = projection.flatten(self);
                let len = projection.len();
                if len == 1 {
                    self.unify_value_type_with_one_col_projection(rhs, projection[0].ty.clone())
                } else {
                    Err(TypeError::Conflict(
                        "cannot unify value type with projection of more than one column"
                            .to_string(),
                    ))
                }
            }

            // All native types are considered equal in the type system.  However, for improved test readability the
            // unifier favours a `NativeValue(Some(_))` over a `NativeValue(None)` because `NativeValue(Some(_))`
            // carries more information. In a tie, the left hand side wins.
            (
                Type::Constructor(Value(Native(native_lhs))),
                Type::Constructor(Value(Native(native_rhs))),
            ) => match (native_lhs, native_rhs) {
                (NativeValue(Some(_)), NativeValue(Some(_))) => Ok(lhs),
                (NativeValue(Some(_)), NativeValue(None)) => Ok(lhs),
                (NativeValue(None), NativeValue(Some(_))) => Ok(rhs),
                _ => Ok(lhs),
            },

            (Type::Constructor(Value(Eql(_))), Type::Constructor(Value(Eql(_)))) => {
                if lhs == rhs {
                    Ok(lhs)
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
            (_, Type::Var(tvar)) => self.unify_with_type_var(lhs, *tvar),

            // A constructor resolves with a type variable if either:
            // 1. the type variable does not already refer to a constructor (transitively), or
            // 2. it does refer to a constructor and the two constructors unify
            (Type::Var(tvar), _) => self.unify_with_type_var(rhs, *tvar),

            // Any other combination of types is a type error.
            (lhs, rhs) => Err(TypeError::Conflict(format!(
                "type {} cannot be unified with {}",
                lhs, rhs
            ))),
        };

        self.depth -= 1;

        match unification {
            Ok(ty) => {
                let span_end = span!(
                    parent: &span_begin,
                    Level::DEBUG,
                    "Ok",
                    unification = ?ty,
                );

                let _guard = span_end.enter();

                Ok(ty)
            }
            Err(err) => {
                let span_end = span!(
                    parent: &span_begin,
                    Level::DEBUG,
                    "Err",
                    unification = ?&err
                );

                let _guard = span_end.enter();

                Err(err)
            }
        }
    }

    /// Unifies a type with a type variable.
    ///
    /// Attempts to unify the type with whatever the type variable is pointing to.
    ///
    /// After successful unification `ty_rc` and `tvar_rc` will refer to the same allocation.
    fn unify_with_type_var(&mut self, ty: Arc<Type>, tvar: TypeVar) -> Result<Arc<Type>, TypeError> {
        let sub_ty = {
            let reg = &*self.registry.borrow();
            reg.get_substitution(tvar)
        };

        let unified_ty: Arc<Type> = match sub_ty {
            Some(sub_ty) => self.unify(ty, sub_ty)?,
            None => Type::from(tvar).into(),
        };

        self.registry.borrow_mut().substitute(tvar, unified_ty.clone());

        Ok(unified_ty)
    }

    /// Unifies two projection types.
    fn unify_projections(
        &mut self,
        lhs: impl Into<Arc<Type>>,
        rhs: impl Into<Arc<Type>>,
    ) -> Result<Arc<Type>, TypeError> {
        let lhs = lhs.into();
        let rhs = rhs.into();
        match (&*lhs, &*rhs) {
            (
                Type::Constructor(Constructor::Projection(lhs_projection)),
                Type::Constructor(Constructor::Projection(rhs_projection)),
            ) => {
                let lhs_projection = lhs_projection.flatten(self);
                let rhs_projection = rhs_projection.flatten(self);

                if lhs_projection.len() == rhs_projection.len() {
                    let mut cols: Vec<ProjectionColumn> = Vec::with_capacity(lhs_projection.len());

                    for (lhs_col, rhs_col) in lhs_projection
                        .columns()
                        .iter()
                        .zip(rhs_projection.columns())
                    {
                        let unified_ty = self.unify(lhs_col.ty.clone(), rhs_col.ty.clone())?;
                        cols.push(ProjectionColumn::new(unified_ty, lhs_col.alias.clone()));
                    }

                    let unified_ty =
                        Type::Constructor(Constructor::Projection(Projection::new(cols)));

                    Ok(unified_ty.into())
                } else {
                    Err(TypeError::Conflict(format!(
                        "cannot unify projections {} and {} because they have different numbers of columns",
                        lhs, rhs
                    )))
                }
            }
            (_, _) => Err(TypeError::InternalError(
                "unify_projections expected projection types".to_string(),
            )),
        }
    }

    fn unify_value_type_with_one_col_projection(
        &mut self,
        value_ty: Arc<Type>,
        projection_ty: Arc<Type>,
    ) -> Result<Arc<Type>, TypeError> {
        match (&*value_ty, &*projection_ty) {
            (
                Type::Constructor(Constructor::Value(Value::Eql(lhs))),
                Type::Constructor(Constructor::Value(Value::Eql(rhs))),
            ) if lhs == rhs => Ok(value_ty.clone()),
            (
                Type::Constructor(Constructor::Value(Value::Native(lhs))),
                Type::Constructor(Constructor::Value(Value::Native(rhs))),
            ) => match (lhs, rhs) {
                (NativeValue(Some(_)), NativeValue(Some(_))) => Ok(value_ty.clone()),
                (NativeValue(Some(_)), NativeValue(None)) => Ok(value_ty.clone()),
                (NativeValue(None), NativeValue(Some(_))) => Ok(projection_ty.clone()),
                _ => Ok(value_ty.clone()),
            },
            (
                Type::Constructor(Constructor::Value(Value::Array(lhs))),
                Type::Constructor(Constructor::Value(Value::Array(rhs))),
            ) => {
                let unified_element_ty = self.unify(lhs.clone(), rhs.clone())?;
                let unified_array_ty =
                    Type::Constructor(Constructor::Value(Value::Array(unified_element_ty)));
                Ok(unified_array_ty.into())
            }
            (Type::Constructor(Constructor::Value(Value::Eql(_))), Type::Var(tvar)) => {
                self.unify_with_type_var(value_ty.clone(), *tvar)
            }
            (Type::Var(tvar), Type::Constructor(Constructor::Value(Value::Eql(_)))) => {
                self.unify_with_type_var(projection_ty.clone(), *tvar)
            }
            _ => Err(TypeError::Conflict(format!(
                "value type {} cannot be unified with single column projection of {}",
                value_ty, projection_ty
            ))),
        }
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use crate::unifier::{Constructor::*, NativeValue, ProjectionColumn, Type, TypeVar, Value::*};
    use crate::unifier::{ProjectionColumns, Unifier};
    use crate::{DepMut, TypeRegistry};

    #[test]
    fn eq_native() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        let lhs: Arc<_> = Type::Constructor(Value(Native(NativeValue(None)))).into();
        let rhs: Arc<_> = Type::Constructor(Value(Native(NativeValue(None)))).into();

        assert_eq!(unifier.unify(lhs.clone(), rhs), Ok(lhs));
    }

    #[ignore = "this is addressed in unmerged PR"]
    #[test]
    fn eq_never() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        let lhs: Arc<_> = Type::Constructor(Projection(
            crate::unifier::Projection::Empty,
        )).into();
        let rhs: Arc<_> = Type::Constructor(Projection(
            crate::unifier::Projection::Empty,
        )).into();

        assert_eq!(unifier.unify(lhs.clone(), rhs), Ok(lhs));
    }

    #[test]
    fn constructor_with_var() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        let lhs: Arc<_> = Type::Constructor(Value(Native(NativeValue(None)))).into();
        let rhs: Arc<_> = Type::Var(TypeVar(0)).into();

        assert_eq!(unifier.unify(lhs.clone(), rhs), Ok(lhs));
    }

    #[test]
    fn var_with_constructor() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        let lhs: Arc<_> = Type::Var(TypeVar(0)).into();
        let rhs: Arc<_> = Type::Constructor(Value(Native(NativeValue(None)))).into();

        assert_eq!(unifier.unify(lhs, rhs.clone()), Ok(rhs));
    }

    #[test]
    fn projections_without_wildcards() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        let lhs: Arc<_> = Type::Constructor(Projection(
            crate::unifier::Projection::WithColumns(ProjectionColumns(vec![
                ProjectionColumn::new(Type::Constructor(Value(Native(NativeValue(None)))), None),
                ProjectionColumn::new(Type::Var(TypeVar(0)), None),
            ])),
        )).into();

        let rhs: Arc<_> = Type::Constructor(Projection(
            crate::unifier::Projection::WithColumns(ProjectionColumns(vec![
                ProjectionColumn::new(Type::Var(TypeVar(1)), None),
                ProjectionColumn::new(Type::Constructor(Value(Native(NativeValue(None)))), None),
            ])),
        )).into();

        let unified = unifier.unify(lhs, rhs).unwrap();

        assert_eq!(
            *unified,
            Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
                ProjectionColumns(vec![
                    ProjectionColumn::new(Type::Constructor(Value(Native(NativeValue(None)))), None),
                    ProjectionColumn::new(Type::Constructor(Value(Native(NativeValue(None)))), None),
                ])
            )))
        );
    }

    #[test]
    fn projections_with_wildcards() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));


        let lhs: Arc<_> = Type::Constructor(Projection(
            crate::unifier::Projection::WithColumns(ProjectionColumns(vec![
                ProjectionColumn::new(Type::Constructor(Value(Native(NativeValue(None)))), None),
                ProjectionColumn::new(Type::Constructor(Value(Native(NativeValue(None)))), None),
            ])),
        )).into();

        let cols: Arc<_> = Type::Constructor(Projection(
            crate::unifier::Projection::WithColumns(ProjectionColumns(vec![
                ProjectionColumn::new(Type::Constructor(Value(Native(NativeValue(None)))), None),
                ProjectionColumn::new(Type::Constructor(Value(Native(NativeValue(None)))), None),
            ])),
        )).into();

        // The RHS is a single projection that contains a projection column that contains a projection with two
        // projection columns.  This is how wildcard expansions is represented at the type level.
        let rhs: Arc<_> = Type::Constructor(Projection(
            crate::unifier::Projection::WithColumns(ProjectionColumns(vec![
                ProjectionColumn::new(cols, None),
            ])),
        )).into();

        let unified = unifier.unify(lhs, rhs).unwrap();

        assert_eq!(
            *unified,
            Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
                ProjectionColumns(vec![
                    ProjectionColumn::new(Type::Constructor(Value(Native(NativeValue(None)))), None),
                    ProjectionColumn::new(Type::Constructor(Value(Native(NativeValue(None)))), None),
                ])
            )))
        );
    }
}
