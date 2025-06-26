//! Type definitions for constructing a [`TypeEnv`] and subsequently an [`InstantiatedTypeEnv`] from [`TypeDecl`]s.
//!
//! A `TypeEnv` is an environment containing `TypeDecl`s. A `TypeDecl` is a mirror of [`Type`] but works symbollically
//! and supports being able to define types with dedicated syntax so that constraints can be built declaratively rather
//! than programatically.
#![allow(unused)]

use std::cell::RefCell;
use std::collections::HashSet;
use std::fmt::Display;
use std::hash::Hash;
use std::rc::Rc;
use std::{collections::HashMap, sync::Arc};

use derive_more::derive::Deref;
use sqltk::parser::ast::{Top, WindowFrameBound};
use topological_sort::TopologicalSort;
use tracing::{event, instrument, Level};

use crate::unifier::instantiated_type_env::InstantiatedTypeEnv;
use crate::{TypeError, TypeRegistry};

use super::{
    ArrayDecl, EqlTraits, ProjectionColumnDecl, ProjectionDecl, Type, TypeDecl, TypeVar, Unifier,
    VarDecl,
};
use super::{InstantiateType, TVar};

/// A collection of [`TypeDecl`]s.
#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct TypeEnv {
    decls: HashMap<TVar, TypeDecl>,
    tvar_counter: usize,
}

impl TypeEnv {
    pub(crate) fn new() -> Self {
        Self {
            /// The [`TypeDecl`]s in the environment.
            decls: HashMap::new(),
            tvar_counter: 0,
        }
    }

    /// Builds a [`TypeEnv`] and returns it.
    ///
    /// After the supplied closure returns this method clones the resulting `TypeEnv` and attempts to instantiate it in
    /// order to verify that it is well-formed.  If instantiaton is successful then the *uninstantiated* `TypeEnv` is
    /// returned.
    ///
    /// This can be used as a template for initialising [`crate::inference::SqlBinaryOp`] and
    /// [`crate::inference::SqlFunction`] environments during unification.
    #[instrument(
        target = "eql-mapper::TYPE_ENV",
        skip(f),
        level = "trace",
        err(Debug),
        fields(
            return = tracing::field::Empty,
        )
    )]
    pub(crate) fn build<F, Out>(mut f: F) -> Result<(Self, Out), TypeError>
    where
        F: FnOnce(&mut TypeEnv) -> Result<Out, TypeError>,
    {
        let span = tracing::Span::current();

        let result = (|| {
            let mut type_env = TypeEnv::new();
            let out = f(&mut type_env)?;
            let cloned = type_env.clone();
            let mut unifier = Unifier::new(Rc::new(RefCell::new(TypeRegistry::new())));
            match cloned.instantiate(&mut unifier) {
                Ok(_) => Ok((type_env, out)),
                Err(err) => Err(err),
            }
        })();

        if let Ok((ref env, _)) = result {
            span.record("return", tracing::field::display(env));
        }

        result
    }

    pub(crate) fn fresh_tvar(&mut self) -> TVar {
        let tvar = TVar(format!("${}", self.tvar_counter));
        self.tvar_counter += 1;
        tvar
    }

    pub(crate) fn add_decl(&mut self, tvar: TVar, spec: TypeDecl) -> TVar {
        self.decls.insert(tvar.clone(), spec);
        tvar
    }

    pub(crate) fn add_decl_with_indirection(&mut self, spec: TypeDecl) -> Result<TVar, TypeError> {
        match spec {
            TypeDecl::Var(VarDecl { tvar, .. }) => {
                self.get_decl(&tvar)?;
                Ok(tvar.clone())
            }
            _ => {
                let tvar = self.fresh_tvar();
                Ok(self.add_decl(tvar, spec))
            }
        }
    }

    pub(crate) fn get_decl(&self, tvar: &TVar) -> Result<&TypeDecl, TypeError> {
        match self.decls.get(tvar) {
            Some(spec) => Ok(spec),
            None => Err(TypeError::InternalError(format!(
                "unknown typespec {} in type env",
                tvar
            ))),
        }
    }

    pub(crate) fn get_bounds(&self, tvar: &TVar) -> Result<EqlTraits, TypeError> {
        match self.decls.get(tvar) {
            Some(TypeDecl::Var(VarDecl { bounds, .. })) => Ok(*bounds),
            Some(_) => Ok(EqlTraits::none()),
            None => Err(TypeError::InternalError(format!(
                "tvar {} not found in type env",
                tvar
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
        event!(
            target: "eql-mapper::TYPE_ENV",
            Level::TRACE,
            type_env = %self,
        );

        let mut tvars = self.tvars_in_order_of_initialisation();

        let mut new_env = InstantiatedTypeEnv::new();

        while let Some(tvar) = tvars.pop() {
            let spec = self
                .decls
                .get(tvar)
                .ok_or(TypeError::InternalError(format!(
                    "expected typespec for tvar {tvar} to be in the typeenv"
                )))?;

            let ty = spec.instantiate_in_env(unifier, &new_env)?;
            new_env.add_type(tvar.clone(), ty);
        }

        Ok(new_env)
    }

    fn tvars_in_order_of_initialisation(&self) -> TopologicalSort<&TVar> {
        let mut topo = TopologicalSort::<&TVar>::new();

        for (tvar, spec) in self.decls.iter() {
            topo.insert(tvar);

            let dependencies = spec.depends_on();

            for dep in dependencies {
                topo.add_dependency(dep, tvar);
            }
        }
        topo
    }
}

impl Display for TypeEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("TypeEnv{ ")?;
        for (idx, (tvar, spec)) in self.decls.iter().enumerate() {
            f.write_fmt(format_args!("{tvar} => {spec}"))?;
            if idx < self.decls.len() - 1 {
                f.write_str(", ")?;
            }
        }
        f.write_str(" }")?;
        Ok(())
    }
}

#[cfg(test)]
mod test {
    use std::{cell::RefCell, rc::Rc, sync::Arc};

    use crate::{
        test_helpers,
        unifier::{
            Array, AssociatedType, EqlTerm, EqlTrait, EqlTraits, EqlValue, InstantiateType, Type,
            Unifier, Value,
        },
        NativeValue, TableColumn, TypeError, TypeRegistry,
    };

    use super::TypeEnv;
    use eql_mapper_macros::{tvar, ty, type_env};
    use pretty_assertions::assert_eq;

    fn make_unifier<'a>() -> Unifier<'a> {
        Unifier::new(Rc::new(RefCell::new(TypeRegistry::new())))
    }

    #[test]
    fn build_env_with_array() -> Result<(), TypeError> {
        let env = type_env! {
            A = [E];
            E = T;
            T = Native;
        };

        let mut unifier = make_unifier();
        let instance = env.instantiate(&mut unifier).unwrap();

        let array_ty = instance.get_type(&tvar!(A))?;

        assert_eq!(&*array_ty, &*ty!([Native]).instantiate_concrete()?);

        Ok(())
    }

    #[test]
    fn build_env_with_projection() -> Result<(), TypeError> {
        let env = type_env! {
            P = {A as id, B as name, C as email};
            A = Native(customer.id);
            B = EQL(customer.name: Eq);
            C = EQL(customer.email: Eq);
        };

        let mut unifier = make_unifier();
        let instance = env.instantiate(&mut unifier).unwrap();

        assert_eq!(
            &*instance.get_type(&tvar!(P))?,
            &*ty!({
                Native(customer.id) as id,
                EQL(customer.name: Eq) as name,
                EQL(customer.email: Eq) as email}
            )
            .instantiate_concrete()?
        );
        Ok(())
    }

    #[test]
    fn build_env_with_associated_type() -> Result<(), TypeError> {
        let env = type_env! {
            E = EQL(customer.name: JsonLike);
            A = <E as JsonLike>::Accessor;
        };

        let mut unifier = make_unifier();
        let instance = env.instantiate(&mut unifier).unwrap();

        if let Type::Associated(associated) = &*instance.get_type(&tvar!(A))? {
            assert_eq!(
                associated.resolve_selector_target(&mut unifier)?.as_deref(),
                Some(&Type::Value(Value::Eql(EqlTerm::JsonAccessor(EqlValue(
                    TableColumn {
                        table: "customer".into(),
                        column: "name".into()
                    },
                    EqlTraits::from(EqlTrait::JsonLike)
                ),))))
            );
        } else {
            panic!("expected associated type");
        }

        Ok(())
    }
}
