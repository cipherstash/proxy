use std::{fmt::Display, sync::Arc};

use derive_more::derive::{Deref, Display};
use sqltk::parser::ast::{BinaryOperator, ObjectName};

use crate::TypeError;

use super::{
    AssociatedType, Constructor, EqlTerm, EqlTraits, InstantiatedTypeEnv, NativeValue, Projection,
    ProjectionColumn, ProjectionColumns, TableColumn, Type, TypeEnv, Unifier, Value,
};

/// A `TypeSpec` is a symbolic placeholder for a [`Type`] and can represent a concrete type (for example: `Native`) or a
/// generic type such as an array or projection.
#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Display)]
pub(crate) enum TypeSpec {
    /// A type variable.
    #[display("{}", _0)]
    Var(VarSpec),
    /// A native type with an optional table-column.
    #[display("{}", _0)]
    Native(NativeSpec),
    /// An EQL column with zero or more [`EqlTrait`] implementations.
    #[display("{}", _0)]
    Eql(EqlTerm),
    /// An array with a generic element type.
    #[display("{}", _0)]
    Array(ArraySpec),
    /// An projection generic & optionally aliased columns types.
    #[display("{}", _0)]
    Projection(ProjectionSpec),
    /// An associated type of a type.
    #[display("{}", _0)]
    AssociatedType(AssociatedTypeSpec),
}

/// Trait for initialising a [`Type`] from a [`TypeSpec`].
pub(crate) trait InitType {
    /// Initialises a [`Type`].
    fn init_type(&self, env: &TypeEnv, unifier: &mut Unifier<'_>) -> Result<Arc<Type>, TypeError>;
}

impl InitType for TypeSpec {
    fn init_type(&self, env: &TypeEnv, unifier: &mut Unifier<'_>) -> Result<Arc<Type>, TypeError> {
        match self {
            TypeSpec::Var(var_spec) => var_spec.init_type(env, unifier),
            TypeSpec::Native(native_spec) => native_spec.init_type(env, unifier),
            TypeSpec::Eql(eql_term) => eql_term.init_type(env, unifier),
            TypeSpec::Array(array_spec) => array_spec.init_type(env, unifier),
            TypeSpec::Projection(projection_spec) => projection_spec.init_type(env, unifier),
            TypeSpec::AssociatedType(associated_type_spec) => {
                associated_type_spec.init_type(env, unifier)
            }
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Display)]
#[display("[{}]", _0)]
pub(crate) struct ArraySpec(pub(crate) Box<TypeSpec>);

impl InitType for ArraySpec {
    fn init_type(&self, env: &TypeEnv, unifier: &mut Unifier<'_>) -> Result<Arc<Type>, TypeError> {
        Ok(Type::array(self.0.init_type(env, unifier)?))
    }
}

// #[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Display)]
// pub(crate) struct VarSpec(#[display("${}", _0)] pub(crate) String);

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Display)]
#[display("{} where {}", tvar, bounds)]
pub(crate) struct VarSpec {
    pub(crate) tvar: TVar,
    pub(crate) bounds: EqlTraits,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Display)]
pub(crate) struct TVar(#[display("${}", _0)] pub(crate) String);

impl InitType for VarSpec {
    fn init_type(&self, env: &TypeEnv, unifier: &mut Unifier<'_>) -> Result<Arc<Type>, TypeError> {
        let bounds = env.get_bounds(&self.tvar)?;
        let target_spec = env.get_type_spec(&self.tvar);

        let ty = unifier.fresh_bounded_tvar(*bounds);
        if let Some(target_spec) = target_spec {
            let target_ty = target_spec.init_type(env, unifier)?;
            Ok(unifier.unify(ty, target_ty)?)
        } else {
            Ok(ty)
        }
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Deref)]
pub(crate) struct TypeSpecBounds(pub(crate) Vec<Bounded>);

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ProjectionSpec(pub(crate) Vec<ProjectionColumnSpec>);

impl Display for ProjectionSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("{")?;
        for (idx, col) in self.0.iter().enumerate() {
            f.write_fmt(format_args!("{}", col))?;
            if idx < self.0.len() - 1 {
                f.write_str(",")?;
            }
        }
        f.write_str("}")
    }
}

impl InitType for ProjectionSpec {
    fn init_type(&self, env: &TypeEnv, unifier: &mut Unifier<'_>) -> Result<Arc<Type>, TypeError> {
        Ok(Arc::new(Type::Constructor(Constructor::Projection(
            Projection::WithColumns(ProjectionColumns(
                self.0
                    .iter()
                    .map(|col_spec| -> Result<_, TypeError> {
                        Ok(ProjectionColumn::new(
                            col_spec.0.init_type(env, unifier)?,
                            col_spec.1.clone(),
                        ))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
            )),
        ))))
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct ProjectionColumnSpec(
    pub(crate) Box<TypeSpec>,
    pub(crate) Option<sqltk::parser::ast::Ident>,
);

impl Display for ProjectionColumnSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{}", self.0))?;
        if let Some(alias) = &self.1 {
            f.write_fmt(format_args!(" as {}", alias))?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord, Display)]
#[display("{}::{}", parent_tvar, associated_type_name)]
pub(crate) struct AssociatedTypeSpec {
    pub(crate) parent_tvar: TVar,
    pub(crate) associated_type_name: &'static str,
}

impl AssociatedTypeSpec {
    pub(crate) fn depends_on(&self) -> Vec<&TVar> {
        vec![&self.parent_tvar]
    }
}

impl InitType for AssociatedTypeSpec {
    fn init_type(&self, env: &TypeEnv, unifier: &mut Unifier<'_>) -> Result<Arc<Type>, TypeError> {
        // let parent_ty = env.get_type_spec(&self.parent_tvar)?.init_type(env, unifier)?;
        let parent_ty = TypeSpec::Var(VarSpec { tvar: self.parent_tvar.clone(), bounds: EqlTraits::default() }).init_type(env, unifier)?;

        Ok(Arc::new(Type::Associated(AssociatedType {
            parent: parent_ty,
            name: self.associated_type_name,
            associated: unifier.fresh_tvar(),
        })))
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Bounded(pub(crate) TVar, pub(crate) EqlTraits);

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct FunctionSpec {
    /// The function name.
    pub(crate) name: ObjectName,
    /// The specification of this function.
    pub(crate) inner: GeneralizedFunctionSpec,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct BinaryOpSpec {
    /// The binary operator.
    pub(crate) op: BinaryOperator,
    /// The specification of this binary operator as a 2-argument function.
    pub(crate) inner: GeneralizedFunctionSpec,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct GeneralizedFunctionSpec {
    /// The generic args of this function - the generic args are local to this function definition.  The ONLY type
    /// variables allowed to be referenced in `args`, `ret` and `bounds`.
    pub(crate) generic_args: Vec<TVar>,

    /// The argument types.
    pub(crate) args: Vec<TypeSpec>,

    /// The return type.
    pub(crate) ret: TypeSpec,

    /// The bounds ('where' clause).
    pub(crate) bounds: Vec<Bounded>,
}

impl GeneralizedFunctionSpec {
    pub(crate) fn init(
        &self,
        unifier: &mut Unifier<'_>,
        args: &[Arc<Type>],
        ret: Arc<Type>,
    ) -> Result<InstantiatedTypeEnv, TypeError> {

        println!("GENFNSPEC: {:#?}", self);

        self.check_no_undeclared_generic_args()?;

        if args.len() != self.args.len() {
            return Err(TypeError::Expected(format!(
                "incorrect number of arguments; got {}, expected {}",
                args.len(),
                self.args.len()
            )));
        }

        let mut env = TypeEnv::new();

        let mut arg_tvars: Vec<TVar> = vec![];

        for arg in self.args.iter() {
            arg_tvars.push(env.add_spec_anonymously(arg.clone())?);
        }

        let ret_tvar = env.add_spec_anonymously(self.ret.clone())?;

        for Bounded(tvar, traits) in &self.bounds {
            env.add_spec_anonymously(
                TypeSpec::Var(VarSpec {
                    tvar: tvar.clone(),
                    bounds: *traits,
                }),
            )?;
        }

        println!("GENFNSPEC BUILT ENV: {:#?}", env);

        let instantiated_env = env.instantiate(unifier)?;

        for (arg, arg_tvar) in args.into_iter().zip(arg_tvars.into_iter()) {
            unifier.unify(arg.clone(), instantiated_env.get_type(&arg_tvar)?)?;
        }

        unifier.unify(ret.clone(), instantiated_env.get_type(&ret_tvar)?)?;

        Ok(instantiated_env)
    }

    fn check_no_undeclared_generic_args<'a>(&'a self) -> Result<(), TypeError> {
        let is_var = |arg: &'a TypeSpec| {
            if let TypeSpec::Var(VarSpec { tvar, .. }) = arg {
                Some(tvar)
            } else {
                None
            }
        };

        let check_known = |tvar: &TVar| -> Result<(), TypeError> {
            if self.generic_args.contains(tvar) {
                Ok(())
            } else {
                Err(TypeError::InternalError(format!(
                    "use of undeclared type var '{}'",
                    tvar
                )))
            }
        };

        self.args
            .iter()
            .filter_map(is_var)
            .fold(Ok(()), |_, tvar| check_known(tvar))?;
        self.bounds
            .iter()
            .map(|Bounded(tvar, _)| tvar)
            .fold(Ok(()), |_, tvar| check_known(tvar))?;
        if let Some(tvar) = is_var(&self.ret) {
            check_known(tvar)?;
        }

        Ok(())
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct NativeSpec(pub(crate) Option<TableColumn>);

impl Display for NativeSpec {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.0 {
            Some(tc) => f.write_fmt(format_args!("Native({})", tc)),
            None => f.write_fmt(format_args!("Native")),
        }
    }
}

impl InitType for NativeSpec {
    fn init_type(&self, _: &TypeEnv, _: &mut Unifier<'_>) -> Result<Arc<Type>, TypeError> {
        match &self.0 {
            Some(tc) => Ok(Arc::new(Type::Constructor(Constructor::Value(
                Value::Native(NativeValue(Some(tc.clone()))),
            )))),
            None => Ok(Arc::new(Type::Constructor(Constructor::Value(
                Value::Native(NativeValue(None)),
            )))),
        }
    }
}

impl InitType for EqlTerm {
    fn init_type(&self, _: &TypeEnv, _: &mut Unifier<'_>) -> Result<Arc<Type>, TypeError> {
        Ok(Arc::new(Type::Constructor(Constructor::Value(Value::Eql(
            self.clone(),
        )))))
    }
}
