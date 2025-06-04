//! Type definitions for constructing a [`TypeEnv`] and subsequently an [`InstantiatedTypeEnv`] from [`TypeSpec`]s.
//!
//! A `TypeEnv` is an environment containing `TypeSpec`s. A `TypeSpec` is a mirror of [`Type`] but works symbollically
//! and supports being able to define types with dedicated syntax so that constraints can be built declaratively rather
//! than programatically.
#![allow(unused)]

use std::{collections::HashMap, sync::Arc};

use derive_more::derive::Deref;
use proc_macro2::TokenStream;
use syn::parse::{Parse, Parser};
use topological_sort::TopologicalSort;

use crate::TypeError;

use super::{ArraySpec, ProjectionColumnSpec, ProjectionSpec, Type, TypeSpec, Unifier, VarSpec};
use super::{InitType, TVar};

/// A collection of [`TypeSpec`]s and their associated [`Bound`]s.
#[derive(Debug, Clone)]
pub(crate) struct TypeEnv {
    specs: HashMap<TVar, TypeSpec>,
}

#[derive(Debug, Clone, Deref)]
pub(crate) struct InstantiatedTypeEnv {
    env: HashMap<TVar, Arc<Type>>,
}

impl InstantiatedTypeEnv {
    pub(crate) fn get_type(&self, var: &TVar) -> Result<Arc<Type>, TypeError> {
        self.env
            .get(var)
            .cloned()
            .ok_or(TypeError::Expected(format!(
                "expected type spec {} to exist in the instantiated type environment",
                &var
            )))
    }
}

impl TypeSpec {
    fn depends_on<'a>(&'a self, env: &'a TypeEnv) -> Vec<&'a TVar> {
        match self {
            TypeSpec::Var(VarSpec { tvar, .. }) => vec![tvar],

            TypeSpec::Native(_) | TypeSpec::Eql(_) => vec![],

            TypeSpec::AssociatedType(associated_type_spec) => {
                vec![associated_type_spec.depends_on()]
            }

            TypeSpec::Array(ArraySpec(spec)) => spec.depends_on(env),

            TypeSpec::Projection(ProjectionSpec(cols)) => {
                let mut depends: Vec<_> = vec![];
                depends.extend(
                    cols.iter()
                        .flat_map(|ProjectionColumnSpec(spec, _)| spec.depends_on(env)),
                );
                depends
            }
        }
    }

    pub(crate) fn from_tokens(tokens: TokenStream) -> syn::Result<Self> {
        TypeSpec::parse.parse2(tokens)
    }
}

impl TypeEnv {
    pub(crate) fn new() -> Self {
        Self {
            specs: HashMap::new(),
        }
    }

    pub(crate) fn add(&mut self, var: &TVar, spec: TypeSpec) -> Result<(), TypeError> {
        match self.specs.insert(var.clone(), spec) {
            Some(_) => Err(TypeError::InternalError(format!(
                "type var '{}' has already been added to the type env",
                var
            ))),
            None => Ok(()),
        }
    }

    pub(crate) fn get(&self, var: &TVar) -> Result<&TypeSpec, TypeError> {
        match self.specs.get(var) {
            Some(spec) => Ok(spec),
            None => Err(TypeError::InternalError(format!(
                "unknown type var '{}'",
                var
            ))),
        }
    }

    /// Builds an [`InstantiatedTypeEnv`] or fails with a [`TypeError`].
    ///
    /// 1. All referenced type arguments be be defined in the env.
    /// 2. All trait bounds must unify (e.g. the type argument to [`EqlTrait::JsonAccessor`]) must have a
    ///    `EqlTrait::Json` bound.
    pub(crate) fn instantiate(
        &self,
        unifier: &mut Unifier<'_>,
    ) -> Result<InstantiatedTypeEnv, TypeError> {
        // Initialise the TypeSpecs based on their topological sort order.
        let mut topo_sort = TopologicalSort::<&TVar>::new();

        for (var_spec, dependencies) in self
            .specs
            .iter()
            .map(|(var_spec, type_spec)| (var_spec, type_spec.depends_on(self)))
        {
            topo_sort.insert(var_spec);

            for dep in dependencies {
                topo_sort.insert(dep);
                topo_sort.add_dependency(var_spec, dep);
            }
        }

        let mut env: HashMap<TVar, Arc<Type>> = HashMap::new();

        while let Some(tvar) = topo_sort.pop() {
            if let Some(spec) = self.specs.get(tvar) {
                env.insert(tvar.clone(), spec.init_type(self, unifier)?);
            }
        }

        Ok(InstantiatedTypeEnv { env })
    }
}

#[cfg(test)]
mod test {
    use std::{cell::RefCell, rc::Rc, sync::Arc};

    use crate::{
        unifier::{
            Array, AssociatedType, Constructor, EqlTerm, EqlTrait, EqlTraits, EqlValue,
            Type, Unifier, Value,
        },
        NativeValue, TableColumn, TypeError, TypeRegistry,
    };

    use super::TypeEnv;
    use pretty_assertions::assert_eq;
    use syn::parse_quote as ty;

    fn make_unifier<'a>() -> Unifier<'a> {
        Unifier::new(Rc::new(RefCell::new(TypeRegistry::new())))
    }

    #[test]
    fn infer_array() -> Result<(), TypeError> {
        let mut env = TypeEnv::new();

        env.add(&ty!(A), ty!([E]))?;
        env.add(&ty!(E), ty!(T))?;
        env.add(&ty!(T), ty!(Native))?;

        let mut unifier = make_unifier();
        let instance = env.instantiate(&mut unifier).unwrap();

        let array_ty = instance.get_type(&ty!(A))?;

        assert_eq!(
            &*array_ty,
            &Type::Constructor(Constructor::Value(Value::Array(Array(Arc::new(
                Type::native()
            )))))
        );

        Ok(())
    }

    #[test]
    fn infer_projection() -> Result<(), TypeError> {
        let mut env = TypeEnv::new();

        env.add(&ty!(P), ty!({A as id, B as name, C as email}))?;
        env.add(&ty!(A), ty!(Native(customer.id)))?;
        env.add(&ty!(B), ty!(EQL(customer.name: Eq)))?;
        env.add(&ty!(C), ty!(EQL(customer.email: Eq)))?;

        let mut unifier = make_unifier();
        let instance = env.instantiate(&mut unifier).unwrap();

        assert_eq!(
            &*instance.get_type(&ty!(P))?,
            &Type::projection(&[
                (
                    Type::Constructor(Constructor::Value(Value::Native(NativeValue(Some(
                        TableColumn {
                            table: "customer".into(),
                            column: "id".into()
                        }
                    )))))
                    .into(),
                    Some("id".into())
                ),
                (
                    Type::Constructor(Constructor::Value(Value::Eql(EqlTerm::Full(EqlValue(
                        TableColumn {
                            table: "customer".into(),
                            column: "name".into()
                        },
                        EqlTraits::from(EqlTrait::Eq)
                    )))))
                    .into(),
                    Some("name".into())
                ),
                (
                    Type::Constructor(Constructor::Value(Value::Eql(EqlTerm::Full(EqlValue(
                        TableColumn {
                            table: "customer".into(),
                            column: "email".into()
                        },
                        EqlTraits::from(EqlTrait::Eq)
                    )))))
                    .into(),
                    Some("email".into())
                ),
            ])
        );

        Ok(())
    }

    #[test]
    fn infer_associated_type() -> Result<(), TypeError> {
        let mut env = TypeEnv::new();

        env.add(&ty!(E), ty!(EQL(customer.name: Json)))?;
        env.add(&ty!(A), ty!(E::Containment))?;
        env.add(&ty!(F), ty!(A))?;

        let mut unifier = make_unifier();
        let instance = env.instantiate(&mut unifier).unwrap();

        // let unified = unifier.unify(instance.get_type(&ty!(F))?, instance.get_type(&ty!(A))?)?;

        // dbg!(&unified);

        assert_eq!(
            &*instance.get_type(&ty!(F))?,
            &Type::Constructor(Constructor::Value(Value::Eql(EqlTerm::Partial(
                EqlValue(
                    TableColumn {
                        table: "customer".into(),
                        column: "name".into()
                    },
                    EqlTraits::from(EqlTrait::Json)
                ),
                EqlTraits::from(EqlTrait::Containment)
            ))))
        );

        Ok(())
    }
}
