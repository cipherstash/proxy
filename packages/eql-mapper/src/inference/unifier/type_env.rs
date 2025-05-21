use std::{collections::HashMap, hash::Hash, sync::Arc};

use derive_more::derive::Deref;

use crate::TypeError;

use super::{Bounds, EqlTrait, Type, Unifier};

/// A `TypeArg` is a symbolic placeholder for a [`Type`] and can represent a concrete type (i.e. `Native`) or a generic type.
///
/// A `TypeArg` can be associated with one or more [`TraitBound`]s when added to a [`TypeEnv`].
///
/// There is no enum variant for an `EqlTerm` concrete type because there are no SQL functions/operators that operate on
/// specific EQL types: the trait bounds mechanism handles this case for us.
#[derive(Debug, Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord, derive_more::Display)]
pub(crate) enum TypeArg {
    /// A placeholder representing the native type.
    #[display("Native")]
    Native,

    /// A generic type.
    #[display("{}", _0)]
    Generic(&'static str),
}

/// Represents the type arguments (if any) required to fully define a trait signature.
///
/// A `TraitBound` bridges between *symbolic* type representations and [`Type::Var`]s, which only exist during a
/// unification run.
#[derive(Debug, Clone, Hash, PartialEq, Eq)]
pub(crate) enum TraitBound {
    /// A bound for a trait that needs no type arguments (e.g. [`EqlTrait::Eq`]).
    WithoutParam(EqlTrait),

    /// A bound for a trait that needs one type argument (e.g. [`EqlTrait::JsonAccessor`]).  The [`TypeArg`] is a type
    /// argument in a [`TypeEnv`] (which may have its own bounds).  The function is used to build the [`EqlTrait`] after
    /// the `TypeArg` has been assigned a [`Type::Var`].
    WithOneParam(TypeArg, fn(Arc<Type>) -> EqlTrait),
}

/// A collection of [`TypeArg`]s and their associated [`TraitBound`]s.
#[derive(Debug, Clone, Deref)]
pub(crate) struct TypeEnv {
    env: HashMap<TypeArg, Vec<TraitBound>>,
}

#[derive(Debug, Clone, Deref)]
pub(crate) struct InstantiatedTypeEnv {
    env: HashMap<TypeArg, Arc<Type>>,
}

impl TypeEnv {
    pub(crate) fn new() -> Self {
        Self {
            env: HashMap::new(),
        }
    }

    pub(crate) fn add_type_arg(&mut self, arg: TypeArg, bound: Option<TraitBound>) {
        let for_insert = bound.clone().map(|bound| vec![bound]).unwrap_or(vec![]);
        self.env
            .entry(arg)
            .and_modify(move |bounds| {
                if let Some(bound) = bound {
                    if !bounds.contains(&bound) {
                        bounds.push(bound)
                    }
                }
            })
            .or_insert(for_insert);
    }

    pub(crate) fn get_bounds(&self, arg: &TypeArg) -> Result<&Vec<TraitBound>, TypeError> {
        match self.env.get(arg) {
            Some(bounds) => Ok(bounds),
            None => Err(TypeError::InternalError(format!(
                "Undeclared type argument `{}`",
                arg
            ))),
        }
    }

    /// Tries to instantiate a well-formed type environment.
    ///
    /// "well-formed" means:
    ///
    /// 1. All referenced type arguments be be defined in the env.
    /// 2. All trait bounds must unify (e.g. the type argument to [`EqlTrait::JsonAccessor`]) must have a
    ///    `EqlTrait::Json` bound.
    pub(crate) fn instantiate(
        &self,
        unifier: &mut Unifier<'_>,
    ) -> Result<InstantiatedTypeEnv, TypeError> {
        // First pass: initialise unbound type variables.
        let types_first_pass: HashMap<TypeArg, Arc<Type>> =
            HashMap::from_iter(self.env.keys().copied().map(|type_arg| match type_arg {
                TypeArg::Native => (type_arg, Arc::new(Type::native())),
                TypeArg::Generic(_) => (type_arg, unifier.fresh_tvar()),
            }));

        // Second pass: initialise new type variables with bounds.
        let types_second_pass: HashMap<TypeArg, Arc<Type>> =
            HashMap::from_iter(types_first_pass.iter().map(|(type_arg, _)| {
                match type_arg {
                    TypeArg::Native => (*type_arg, Arc::new(Type::native())),
                    TypeArg::Generic(_) => {
                        let mut bounds = Bounds::None;
                        for bound in self.get_bounds(type_arg).unwrap() {
                            match bound {
                                TraitBound::WithoutParam(eql_trait) => {
                                    bounds = bounds.union(&Bounds::from(eql_trait.clone()))
                                }
                                TraitBound::WithOneParam(param, ctor) => {
                                    bounds = bounds.union(&Bounds::from(ctor(
                                        types_first_pass[param].clone(),
                                    )))
                                }
                            };
                        }

                        (*type_arg, unifier.fresh_bounded_tvar(bounds))
                    },
                }
            }));

        // Third pass: unify the corresponding type variables
        let mut env: HashMap<TypeArg, Arc<Type>> = HashMap::new();

        for (type_arg, ty_a) in types_first_pass {
            let ty_b = &types_second_pass[&type_arg];
            let ty_unified = unifier.unify(ty_a.clone(), ty_b.clone())?;
            env.insert(type_arg, ty_unified);
        }

        Ok(InstantiatedTypeEnv { env })
    }
}
