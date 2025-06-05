//! Type definitions for constructing a [`TypeEnv`] and subsequently an [`InstantiatedTypeEnv`] from [`TypeSpec`]s.
//!
//! A `TypeEnv` is an environment containing `TypeSpec`s. A `TypeSpec` is a mirror of [`Type`] but works symbollically
//! and supports being able to define types with dedicated syntax so that constraints can be built declaratively rather
//! than programatically.
#![allow(unused)]

use std::hash::Hash;
use std::{collections::HashMap, sync::Arc};

use derive_more::derive::Deref;
use proc_macro2::TokenStream;
use sqltk::parser::ast::Top;
use syn::parse::{Parse, Parser};
use topological_sort::TopologicalSort;

use crate::TypeError;

use super::{
    ArraySpec, EqlTraits, ProjectionColumnSpec, ProjectionSpec, Type, TypeSpec, TypeVar, Unifier,
    VarSpec,
};
use super::{InitType, TVar};

/// A collection of [`TypeSpec`]s.
#[derive(Debug, Clone)]
pub(crate) struct TypeEnv {
    tvar_to_spec: HashMap<TVar, TypeSpec>,
    tvar_bounds: HashMap<TVar, EqlTraits>,
    tvar_counter: usize,
}

impl TypeSpec {
    fn depends_on<'a>(&'a self, env: &'a TypeEnv) -> Vec<&'a TVar> {
        match self {
            TypeSpec::Var(VarSpec { tvar, .. }) => vec![tvar],

            TypeSpec::Native(_) | TypeSpec::Eql(_) => vec![],

            TypeSpec::AssociatedType(associated_type_spec) => associated_type_spec.depends_on(),

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
            tvar_to_spec: HashMap::new(),
            tvar_bounds: HashMap::new(),
            tvar_counter: 0,
        }
    }

    pub(crate) fn fresh_tvar(&mut self) -> TVar {
        self.tvar_counter += 1;
        TVar(format!("${}", self.tvar_counter))
    }

    pub(crate) fn add_spec(&mut self, tvar: TVar, spec: TypeSpec) -> Result<(), TypeError> {
        eprintln!("added: {} = {}", tvar, &spec);

        self.tvar_to_spec.insert(tvar.clone(), spec);
        self.tvar_bounds.entry(tvar.clone()).or_default();

        Ok(())
    }

    pub(crate) fn add_spec_anonymously(&mut self, spec: TypeSpec) -> Result<TVar, TypeError> {
        if let TypeSpec::Var(VarSpec { tvar, bounds }) = spec {
            if let Some(existing_bounds) = self.tvar_bounds.get(&tvar) {
                self.tvar_bounds
                    .insert(tvar.clone(), bounds.union(&existing_bounds));
            } else {
                self.tvar_bounds.insert(tvar.clone(), bounds);
            }
            Ok(tvar)
        } else {
            let tvar = self.fresh_tvar();
            self.add_spec(tvar.clone(), spec);
            Ok(tvar)
        }
    }

    pub(crate) fn get_type_spec(&self, var: &TVar) -> Option<&TypeSpec> {
        self.tvar_to_spec.get(var)
    }

    pub(crate) fn get_bounds(&self, var: &TVar) -> Result<&EqlTraits, TypeError> {
        match self.tvar_bounds.get(var) {
            Some(bounds) => Ok(bounds),
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

        for (tvar, dependencies) in self
            .tvar_to_spec
            .iter()
            .map(|(tvar, type_spec)| (tvar, type_spec.depends_on(self)))
        {
            topo_sort.insert(tvar);

            for tvar_dep in dependencies {
                topo_sort.insert(tvar_dep);
                topo_sort.add_dependency(tvar, tvar_dep);
            }
        }

        for (tvar, _) in &self.tvar_bounds {
            topo_sort.insert(tvar);
        }

        let mut env: HashMap<TVar, Arc<Type>> = HashMap::new();

        while let Some(tvar) = topo_sort.pop() {
            eprintln!("Adding {tvar} to env");
            let spec = VarSpec {
                tvar: tvar.clone(),
                bounds: EqlTraits::default(),
            };

            env.insert(tvar.clone(), spec.init_type(self, unifier)?);
        }

        eprintln!("Env size is {}", env.len());

        Ok(InstantiatedTypeEnv { env })
    }
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

#[cfg(test)]
mod test {
    use std::{cell::RefCell, rc::Rc, sync::Arc};

    use crate::{
        unifier::{
            Array, AssociatedType, Constructor, EqlTerm, EqlTrait, EqlTraits, EqlValue, Type,
            Unifier, Value,
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

        env.add_spec(ty!(A), ty!([E]))?;
        env.add_spec(ty!(E), ty!(T))?;
        env.add_spec(ty!(T), ty!(Native))?;

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

        env.add_spec(ty!(P), ty!({A as id, B as name, C as email}))?;
        env.add_spec(ty!(A), ty!(Native(customer.id)))?;
        env.add_spec(ty!(B), ty!(EQL(customer.name: Eq)))?;
        env.add_spec(ty!(C), ty!(EQL(customer.email: Eq)))?;

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

        env.add_spec(ty!(E), ty!(EQL(customer.name: Json)))?;
        env.add_spec(ty!(A), ty!(E::Containment))?;
        env.add_spec(ty!(F), ty!(A))?;

        let mut unifier = make_unifier();
        let instance = env.instantiate(&mut unifier).unwrap();

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
