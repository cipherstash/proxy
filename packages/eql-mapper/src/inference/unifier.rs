use std::{cell::RefCell, collections::HashMap, rc::Rc};

use crate::{inference::TypeError, Def, Status, TypeVar};

use super::{Constructor, ProjectionColumn, Type, TypeVarGenerator};

/// Implements the type unification algorithm and maintains an association of type variables with the type that they
/// point to.
#[derive(Debug)]
pub struct Unifier {
    /// A map of type variable substitutions.
    subs: HashMap<u32, Rc<RefCell<Type>>>,
    tvar_gen: TypeVarGenerator,
}

impl Default for Unifier {
    fn default() -> Self {
        Self::new()
    }
}

impl Unifier {
    /// Creates a new `Unifier`.
    pub fn new() -> Self {
        Self {
            subs: HashMap::new(),
            tvar_gen: TypeVarGenerator::new(),
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
    ///
    /// After successful unification, `left` and `right` will refer to the same allocation.
    pub fn unify(
        &mut self,
        left: Rc<RefCell<Type>>,
        right: Rc<RefCell<Type>>,
    ) -> Result<Rc<RefCell<Type>>, TypeError> {
        use crate::Constructor::*;
        use crate::Def::*;

        let (a, b) = (left.borrow(), right.borrow());

        match (&*a, &*b) {
            // Two projections unify if they have the same number of columns and all of the paired column types also
            // unify.
            (
                Type(Constructor(Projection(cols_left)), _),
                Type(Constructor(Projection(cols_right)), _),
            ) => {
                let projection_columns =
                    self.unify_projection(cols_left.clone(), cols_right.clone())?;

                drop(a);
                drop(b);

                let resolved = projection_columns
                    .borrow()
                    .iter()
                    .fold(Status::Resolved, |acc, col| col.ty.borrow().status() + acc);

                *left.borrow_mut() = Type(Constructor(Projection(projection_columns)), resolved);

                *right.borrow_mut() = left.borrow().clone();

                Ok(left.clone())
            }

            // For types that are resolved, in order to successfully unify they must either be:
            // - equal (according to the Eq trait), or
            // - both be native
            (Type(body_a, Status::Resolved), Type(body_b, Status::Resolved)) => {
                if body_a == body_b {
                    Ok(left.clone())
                } else {
                    match (body_a, body_b) {
                        // Constructor::AnonymousNative and Constructor::Scalar(Scalar::Native{ .. }) will unify
                        // to Constructor::Scalar(Scalar::Native{ .. }) to preserve information.
                        (Def::Constructor(ctor_a), Def::Constructor(ctor_b))
                            if ctor_a.is_native() && ctor_b.is_native() =>
                        {
                            if let Scalar(_) = ctor_a {
                                return Ok(left.clone());
                            }

                            if let Scalar(_) = ctor_b {
                                return Ok(right.clone());
                            }

                            Ok(left.clone())
                        }

                        _ => Err(TypeError::Conflict(format!(
                            "expected resolved types {} and {} to unify",
                            a, b
                        ))),
                    }
                }
            }

            // If a type is a fresh type variable then assign it a unique identifier before continuing.
            (&Type(Var(TypeVar::Fresh), _), _) => {
                drop(a);
                drop(b);

                *left.borrow_mut() = Type(
                    Def::Var(TypeVar::Assigned(self.tvar_gen.next_tvar())),
                    Status::Partial,
                );

                Ok(self.unify(left, right)?)
            }

            // If a type is a fresh type variable then assign it a unique identifier before continuing.
            (_, &Type(Var(TypeVar::Fresh), _)) => {
                drop(a);
                drop(b);

                *right.borrow_mut() = Type(
                    Def::Var(TypeVar::Assigned(self.tvar_gen.next_tvar())),
                    Status::Partial,
                );

                Ok(self.unify(left, right)?)
            }

            // Two arrays unify if the types of their elements unify.
            (
                Type(Constructor(Array(element_ty_left)), _),
                Type(Constructor(Array(element_ty_right)), _),
            ) => {
                let element_ty = self.unify(element_ty_left.clone(), element_ty_right.clone())?;

                drop(a);
                drop(b);

                *left.borrow_mut() = Type(
                    Constructor(Array(element_ty.clone())),
                    element_ty.borrow().status(),
                );

                *right.borrow_mut() = left.borrow().clone();

                Ok(left.clone())
            }

            // A constructor resolves with a type variable if either:
            // 1. the type variable does not already refer to a constructor (transitively), or
            // 2. it does refer to a constructor and the two constructors unify
            (Type(_, _), &Type(Var(TypeVar::Assigned(tvar)), _)) => {
                drop(a);
                drop(b);

                Ok(self.unify_with_type_var(left, right, tvar)?)
            }

            // A constructor resolves with a type variable if either:
            // 1. the type variable does not already refer to a constructor (transitively), or
            // 2. it does refer to a constructor and the two constructors unify
            (&Type(Var(TypeVar::Assigned(tvar)), _), Type(_, _)) => {
                drop(a);
                drop(b);

                Ok(self.unify_with_type_var(right, left, tvar)?)
            }

            // Any other combination of types is a type error.
            (left_ty, right_ty) => Err(TypeError::Conflict(format!(
                "type {} cannot be unified with {}",
                left_ty, right_ty
            ))),
        }
    }

    /// Unifies a type with a type variable.
    ///
    /// Attempts to unify the type with whatever the type variable is pointing to.
    ///
    /// After successful unification `ty_rc` and `tvar_rc` will refer to the same allocation.
    fn unify_with_type_var(
        &mut self,
        ty_rc: Rc<RefCell<Type>>,
        tvar_rc: Rc<RefCell<Type>>,
        tvar: u32,
    ) -> Result<Rc<RefCell<Type>>, TypeError> {
        if let Some(sub_ty) = self.subs.get(&tvar).cloned() {
            self.unify(ty_rc.clone(), sub_ty)?;
        }

        self.subs.insert(tvar, ty_rc.clone());

        *tvar_rc.borrow_mut() = ty_rc.borrow().clone();

        Ok(ty_rc.clone())
    }

    /// Unifies two `Vec`s of [`ProjectionColumn`].
    ///
    /// The same number of columns must be present in `left` and `right` (after flattening out nested projections due to
    /// use of wildcards) and all corresponding pairs of columns must unify.
    ///
    /// After successfull unification `left` and `right` will refer to the same allocation.
    fn unify_projection(
        &mut self,
        left: Rc<RefCell<Vec<ProjectionColumn>>>,
        right: Rc<RefCell<Vec<ProjectionColumn>>>,
    ) -> Result<Rc<RefCell<Vec<ProjectionColumn>>>, TypeError> {
        {
            Self::flatten_projection(left.clone());
            Self::flatten_projection(right.clone());

            let cols_left_mut = &mut *left.borrow_mut();
            let cols_right_mut = &mut *right.borrow_mut();

            if cols_left_mut.len() == cols_right_mut.len() {
                for (col_a, col_b) in cols_left_mut.iter_mut().zip(cols_right_mut.iter_mut()) {
                    *col_a = self.unify_projection_columns(col_a, col_b)?;
                }

                *cols_right_mut = cols_left_mut.clone();

                return Ok(left.clone());
            }
        }

        Err(TypeError::Conflict(format!(
            "cannot unify projections because column counts are different:\n{}\n{}",
            Self::render_projection(left.clone()),
            Self::render_projection(right.clone())
        )))
    }

    fn flatten_projection(projection: Rc<RefCell<Vec<ProjectionColumn>>>) {
        let cols = projection.borrow();
        let mut flattened: Vec<ProjectionColumn> = Vec::with_capacity(cols.len());

        for idx in 0..cols.len() {
            let col = &cols[idx];
            match &*col.ty.borrow() {
                Type(Def::Constructor(Constructor::Projection(inner_cols)), _) => {
                    Self::flatten_projection(inner_cols.clone());
                    flattened.extend(inner_cols.borrow().iter().cloned());
                }
                _ => flattened.push(col.clone()),
            }
        }

        drop(cols);
        *projection.borrow_mut() = flattened;
    }

    /// Unifies a `left` and  `right` [`ProjectionColumn`].
    ///
    /// In order to unified the `ty` of each `ProjectionColumn` must unify and their `alias` must also unify.  Aliases
    /// unify if either both aliases are `None`, one of the aliases is `Some(_)` or both are `Some(_)` and equal.
    ///
    /// After successful unification, the `ty` of `left` and `right` will refer to the same allocation.
    fn unify_projection_columns(
        &mut self,
        left: &ProjectionColumn,
        right: &ProjectionColumn,
    ) -> Result<ProjectionColumn, TypeError> {
        let ty = self.unify(left.ty.clone(), right.ty.clone())?;

        match (&left.alias, &right.alias) {
            (None, None) => Ok(ProjectionColumn { ty, alias: None }),

            (None, Some(alias)) => Ok(ProjectionColumn {
                ty,
                alias: Some(alias.clone()),
            }),

            (Some(alias), None) => Ok(ProjectionColumn {
                ty,
                alias: Some(alias.clone()),
            }),

            (Some(a), Some(b)) if a == b => Ok(ProjectionColumn {
                ty,
                alias: Some(a.clone()),
            }),

            (Some(a), Some(b)) => Err(TypeError::Conflict(format!(
                "projection column aliases are not equal: {} and {}",
                a, b
            ))),
        }
    }

    pub(crate) fn render_projection(projection: Rc<RefCell<Vec<ProjectionColumn>>>) -> String {
        let projection = &*projection.borrow();
        let ty_strings: Vec<String> = projection
            .iter()
            .map(|col| col.ty.borrow().to_string())
            .collect();
        format!("[{}]", ty_strings.join(", "))
    }
}

#[cfg(test)]
mod test {
    use crate::inference::types::{
        Constructor::*, Def::*, ProjectionColumn, Scalar::*, Status, Type, TypeVar,
    };
    use crate::Unifier;
    use std::{cell::RefCell, rc::Rc};

    #[test]
    fn eq_native() {
        let left = Type(
            Constructor(Scalar(Rc::new(AnonymousNative))),
            Status::Resolved,
        )
        .wrap();
        let right = Type(
            Constructor(Scalar(Rc::new(AnonymousNative))),
            Status::Resolved,
        )
        .wrap();

        let mut unifier = Unifier::new();

        assert_eq!(unifier.unify(left.clone(), right.clone()), Ok(left.clone()));
    }

    #[test]
    fn eq_never() {
        let left = Type(Constructor(Empty), Status::Resolved).wrap();
        let right = Type(Constructor(Empty), Status::Resolved).wrap();

        let mut unifier = Unifier::new();

        assert_eq!(unifier.unify(left.clone(), right.clone()), Ok(left.clone()));
    }

    #[test]
    fn constructor_with_var() {
        let left = Type(
            Constructor(Scalar(Rc::new(AnonymousNative))),
            Status::Resolved,
        )
        .wrap();
        let right = Type(Var(TypeVar::Fresh), Status::Partial).wrap();

        let mut unifier = Unifier::new();

        assert_eq!(unifier.unify(left.clone(), right.clone()), Ok(left.clone()));
        assert_eq!(right, left);
        assert_eq!(right.borrow().status(), Status::Resolved);
    }

    #[test]
    fn var_with_constructor() {
        let left = Type(Var(TypeVar::Fresh), Status::Partial).wrap();
        let right = Type(
            Constructor(Scalar(Rc::new(AnonymousNative))),
            Status::Resolved,
        )
        .wrap();

        let mut unifier = Unifier::new();

        assert_eq!(unifier.unify(left.clone(), right.clone()), Ok(left.clone()));
        assert_eq!(right, left);
        assert_eq!(left.borrow().status(), Status::Resolved);
    }

    #[test]
    fn projections_without_wildcards() {
        let left = Type(
            Constructor(Projection(Rc::new(RefCell::new(vec![
                ProjectionColumn {
                    ty: Type(
                        Constructor(Scalar(Rc::new(AnonymousNative))),
                        Status::Resolved,
                    )
                    .wrap(),
                    alias: None,
                },
                ProjectionColumn {
                    ty: Type(Var(TypeVar::Fresh), Status::Partial).wrap(),
                    alias: None,
                },
            ])))),
            Status::Partial,
        )
        .wrap();

        let right = Type(
            Constructor(Projection(Rc::new(RefCell::new(vec![
                ProjectionColumn {
                    ty: Type(Var(TypeVar::Fresh), Status::Partial).wrap(),
                    alias: None,
                },
                ProjectionColumn {
                    ty: Type(Constructor(Empty), Status::Resolved).wrap(),
                    alias: None,
                },
            ])))),
            Status::Partial,
        )
        .wrap();

        let mut unifier = Unifier::new();

        assert_eq!(
            unifier.unify(left.clone(), right.clone()),
            Ok(Type(
                Constructor(Projection(Rc::new(RefCell::new(vec![
                    ProjectionColumn {
                        ty: Type(
                            Constructor(Scalar(Rc::new(AnonymousNative))),
                            Status::Resolved
                        )
                        .wrap(),
                        alias: None
                    },
                    ProjectionColumn {
                        ty: Type(Constructor(Empty), Status::Resolved).wrap(),
                        alias: None
                    },
                ])))),
                Status::Resolved
            )
            .wrap())
        );

        assert_eq!(right, left);
    }

    #[test]
    fn projections_with_wildcards() {
        let left = Type(
            Constructor(Projection(Rc::new(RefCell::new(vec![
                ProjectionColumn {
                    ty: Type(
                        Constructor(Scalar(Rc::new(AnonymousNative))),
                        Status::Resolved,
                    )
                    .wrap(),
                    alias: None,
                },
                ProjectionColumn {
                    ty: Type(Constructor(Empty), Status::Resolved).wrap(),
                    alias: None,
                },
            ])))),
            Status::Resolved,
        )
        .wrap();

        // The RHS is a single projection that contains a projection column that contains a projection with two
        // projection columns.  This is how wildcard expansions is represented at the type level. Unification should
        // flatten a nested projection.
        let right = Type(
            Constructor(Projection(Rc::new(RefCell::new(vec![ProjectionColumn {
                ty: Type(
                    Constructor(Projection(Rc::new(RefCell::new(vec![
                        ProjectionColumn {
                            ty: Type(
                                Constructor(Scalar(Rc::new(AnonymousNative))),
                                Status::Resolved,
                            )
                            .wrap(),
                            alias: None,
                        },
                        ProjectionColumn {
                            ty: Type(Constructor(Empty), Status::Resolved).wrap(),
                            alias: None,
                        },
                    ])))),
                    Status::Resolved,
                )
                .wrap(),
                alias: None,
            }])))),
            Status::Resolved,
        )
        .wrap();

        let mut unifier = Unifier::new();

        assert_eq!(
            unifier.unify(left.clone(), right.clone()),
            Ok(Type(
                Constructor(Projection(Rc::new(RefCell::new(vec![
                    ProjectionColumn {
                        ty: Type(
                            Constructor(Scalar(Rc::new(AnonymousNative))),
                            Status::Resolved
                        )
                        .wrap(),
                        alias: None
                    },
                    ProjectionColumn {
                        ty: Type(Constructor(Empty), Status::Resolved).wrap(),
                        alias: None
                    },
                ])))),
                Status::Resolved
            )
            .wrap())
        );

        assert_eq!(right, left);
    }
}
