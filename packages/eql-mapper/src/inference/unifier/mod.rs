use std::{cell::RefCell, rc::Rc};

mod types;

use crate::inference::TypeError;

use sqltk::Visitable;
pub(crate) use types::*;

pub use types::{EqlValue, NativeValue, TableColumn};

use super::{TypeRegistry, TID};
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

    /// Registers a [`Type`] and returns its [`TID`].
    pub(crate) fn register(&self, ty: Type) -> TID {
        self.registry.borrow_mut().register(ty)
    }

    /// Looks up a previously registered [`Type`] by its [`TID`].
    pub(crate) fn lookup(&self, tid: TID) -> Type {
        self.registry.borrow().get_type_by_tid(tid)
    }

    pub(crate) fn lookup_node_tid<N: Visitable>(&self, node: &'ast N) -> TID {
        self.registry.borrow().get_node_tid(node)
    }

    pub(crate) fn lookup_substitution(&self, tvar: TypeVar) -> TID {
        self.registry.borrow().get_substitution(tvar)
    }

    pub(crate) fn exists_node_with_type<N: Visitable>(&self, ty: &Type) -> bool {
        self.registry.borrow().exists_node_with_type::<N>(ty)
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
    pub(crate) fn unify(&mut self, lhs_tid: TID, rhs_tid: TID) -> Result<TID, TypeError> {
        use types::Constructor::*;
        use types::Value::*;

        let span = span!(
            Level::DEBUG,
            "unify",
            depth = self.depth,
            // lhs = lhs,
            // rhs = rhs,
        );

        let _guard = span.enter();

        self.depth += 1;

        // Short-circuit the unification when lhs & rhs are equal.
        if lhs_tid == rhs_tid {
            return Ok(lhs_tid);
        }

        let lhs = self.registry.borrow_mut().get_type_by_tid(lhs_tid);
        let rhs = self.registry.borrow_mut().get_type_by_tid(rhs_tid);

        let unification = match (&lhs, &rhs) {
            // Two projections unify if they have the same number of columns and all of the paired column types also
            // unify.
            (Type::Constructor(Projection(_)), Type::Constructor(Projection(_))) => {
                self.unify_projections(&lhs, &rhs)
            }

            // Two arrays unify if the types of their element types unify.
            (
                Type::Constructor(Value(Array(lhs_element_tid))),
                Type::Constructor(Value(Array(rhs_element_tid))),
            ) => {
                let unified_element_tid = self.unify(*lhs_element_tid, *rhs_element_tid)?;
                let unified_array_tid = self
                    .registry
                    .borrow_mut()
                    .register(Type::Constructor(Value(Array(unified_element_tid))));

                Ok(unified_array_tid)
            }

            // A Value can unify with a single column projection
            (Type::Constructor(Value(_)), Type::Constructor(Projection(projection))) => {
                let projection = projection.flatten(&self.registry.borrow());
                let len = projection.len();
                if len == 1 {
                    self.unify_value_type_with_type(lhs_tid, projection[0].tid)
                } else {
                    Err(TypeError::Conflict(
                        "cannot unify value type with projection of more than one column"
                            .to_string(),
                    ))
                }
            }

            (Type::Constructor(Projection(projection)), Type::Constructor(Value(_))) => {
                let projection = projection.flatten(&self.registry.borrow());
                let len = projection.len();
                if len == 1 {
                    self.unify_value_type_with_type(rhs_tid, projection[0].tid)
                } else {
                    Err(TypeError::Conflict(
                        "cannot unify value type with projection of more than one column"
                            .to_string(),
                    ))
                }
            }

            (Type::Constructor(Value(Native(_))), Type::Constructor(Value(Native(_)))) => {
                Ok(lhs_tid)
            }

            (Type::Constructor(Value(Eql(_))), Type::Constructor(Value(Eql(_)))) => {
                if lhs == rhs {
                    Ok(lhs_tid)
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
            (_, Type::Var(tvar)) => self.unify_with_type_var(lhs_tid, *tvar),

            // A constructor resolves with a type variable if either:
            // 1. the type variable does not already refer to a constructor (transitively), or
            // 2. it does refer to a constructor and the two constructors unify
            (Type::Var(tvar), _) => self.unify_with_type_var(rhs_tid, *tvar),

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
    fn unify_with_type_var(&mut self, tid: TID, tvar: TypeVar) -> Result<TID, TypeError> {
        let sub_tid = {
            let reg = &*self.registry.borrow();
            reg.get_substitution(tvar)
        };

        let unified_tid = self.unify(tid, sub_tid)?;
        self.registry.borrow_mut().substitute(tvar, unified_tid);

        Ok(unified_tid)
    }

    /// Unifies two projection types.
    fn unify_projections(&mut self, lhs: &Type, rhs: &Type) -> Result<TID, TypeError> {
        match (lhs, rhs) {
            (
                Type::Constructor(Constructor::Projection(lhs_projection)),
                Type::Constructor(Constructor::Projection(rhs_projection)),
            ) => {
                let lhs_projection = lhs_projection.flatten(&self.registry.borrow());
                let rhs_projection = rhs_projection.flatten(&self.registry.borrow());

                if lhs_projection.len() == rhs_projection.len() {
                    let mut cols: Vec<ProjectionColumn> = Vec::with_capacity(lhs_projection.len());

                    for (lhs_col, rhs_col) in lhs_projection
                        .columns()
                        .iter()
                        .zip(rhs_projection.columns())
                    {
                        let unified_tid = self.unify(lhs_col.tid, rhs_col.tid)?;
                        cols.push(ProjectionColumn::new(unified_tid, lhs_col.alias.clone()));
                    }

                    let unified_tid = self.registry.borrow_mut().register(Type::Constructor(
                        Constructor::Projection(Projection::new(cols)),
                    ));

                    Ok(unified_tid)
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

    fn unify_value_type_with_type(&mut self, value: TID, ty: TID) -> Result<TID, TypeError> {
        let value_ty = self.registry.borrow().get_type_by_tid(value);
        let ty_ty = self.registry.borrow().get_type_by_tid(ty);
        match (&value_ty, &ty_ty) {
            (
                Type::Constructor(Constructor::Value(Value::Eql(lhs))),
                Type::Constructor(Constructor::Value(Value::Eql(rhs))),
            ) if lhs == rhs => Ok(value),
            (
                Type::Constructor(Constructor::Value(Value::Native(_))),
                Type::Constructor(Constructor::Value(Value::Native(_))),
            ) => Ok(value),
            (
                Type::Constructor(Constructor::Value(Value::Array(lhs))),
                Type::Constructor(Constructor::Value(Value::Array(rhs))),
            ) => {
                let unified_element_tid = self.unify(*lhs, *rhs)?;
                let unified_array_tid =
                    self.registry
                        .borrow_mut()
                        .register(Type::Constructor(Constructor::Value(Value::Array(
                            unified_element_tid,
                        ))));
                Ok(unified_array_tid)
            }
            (Type::Constructor(Constructor::Value(Value::Eql(_))), Type::Var(tvar)) => {
                self.unify_with_type_var(value, *tvar)
            }
            (Type::Var(tvar), Type::Constructor(Constructor::Value(Value::Eql(_)))) => {
                self.unify_with_type_var(value, *tvar)
            }
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
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        let lhs = unifier.register(Type::Constructor(Value(Native(NativeValue(None)))));
        let rhs = unifier.register(Type::Constructor(Value(Native(NativeValue(None)))));

        assert_eq!(unifier.unify(lhs, rhs), Ok(lhs));
    }

    #[ignore = "this is addressed in unmerged PR"]
    #[test]
    fn eq_never() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        let lhs = unifier.register(Type::Constructor(Projection(
            crate::unifier::Projection::Empty,
        )));
        let rhs = unifier.register(Type::Constructor(Projection(
            crate::unifier::Projection::Empty,
        )));

        assert_eq!(unifier.unify(lhs, rhs), Ok(lhs));
    }

    #[test]
    fn constructor_with_var() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        let lhs = unifier.register(Type::Constructor(Value(Native(NativeValue(None)))));
        let rhs = unifier.register(Type::Var(TypeVar(0)));

        assert_eq!(unifier.unify(lhs, rhs), Ok(lhs));
    }

    #[test]
    fn var_with_constructor() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        let lhs = unifier.register(Type::Var(TypeVar(0)));
        let rhs = unifier.register(Type::Constructor(Value(Native(NativeValue(None)))));

        assert_eq!(unifier.unify(lhs, rhs), Ok(lhs));
    }

    #[test]
    fn projections_without_wildcards() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        let native_val = unifier.register(Type::Constructor(Value(Native(NativeValue(None)))));

        let lhs = unifier.register(Type::Constructor(Projection(
            crate::unifier::Projection::WithColumns(ProjectionColumns(vec![
                ProjectionColumn::new(native_val, None),
                ProjectionColumn::new(unifier.register(Type::Var(TypeVar(0))), None),
            ])),
        )));

        let rhs = unifier.register(Type::Constructor(Projection(
            crate::unifier::Projection::WithColumns(ProjectionColumns(vec![
                ProjectionColumn::new(unifier.register(Type::Var(TypeVar(1))), None),
                ProjectionColumn::new(native_val, None),
            ])),
        )));

        let unified = unifier.unify(lhs, rhs).unwrap();

        assert_eq!(
            unifier.lookup(unified),
            Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
                ProjectionColumns(vec![
                    ProjectionColumn::new(native_val, None),
                    ProjectionColumn::new(native_val, None),
                ])
            )))
        );
    }

    #[test]
    fn projections_with_wildcards() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        let native_val = unifier.register(Type::Constructor(Value(Native(NativeValue(None)))));

        let lhs = unifier.register(Type::Constructor(Projection(
            crate::unifier::Projection::WithColumns(ProjectionColumns(vec![
                ProjectionColumn::new(native_val, None),
                ProjectionColumn::new(native_val, None),
            ])),
        )));

        let cols = unifier.register(Type::Constructor(Projection(
            crate::unifier::Projection::WithColumns(ProjectionColumns(vec![
                ProjectionColumn::new(native_val, None),
                ProjectionColumn::new(native_val, None),
            ])),
        )));

        // The RHS is a single projection that contains a projection column that contains a projection with two
        // projection columns.  This is how wildcard expansions is represented at the type level.
        let rhs = unifier.register(Type::Constructor(Projection(
            crate::unifier::Projection::WithColumns(ProjectionColumns(vec![
                ProjectionColumn::new(cols, None),
            ])),
        )));

        let unified = unifier.unify(lhs, rhs).unwrap();

        assert_eq!(
            unifier.lookup(unified),
            Type::Constructor(Projection(crate::unifier::Projection::WithColumns(
                ProjectionColumns(vec![
                    ProjectionColumn::new(native_val, None),
                    ProjectionColumn::new(native_val, None),
                ])
            )))
        );
    }
}
