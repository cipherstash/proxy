use std::{fmt::Display, sync::Arc};

use derive_more::derive::{Deref, Display};
use sqltk::parser::ast::{BinaryOperator, ObjectName};

use crate::TypeError;

use super::{
    AssociatedType, Constructor, EqlTerm, EqlTraits, InstantiatedTypeEnv, JsonQueryType,
    NativeValue, Projection, ProjectionColumn, ProjectionColumns, TableColumn, Type, TypeEnv,
    Unifier, Value,
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
        let ty = unifier.fresh_tvar();
        let other_ty = env.get(&self.tvar)?.init_type(env, unifier)?;
        unifier.unify(ty, other_ty)
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
pub(crate) enum AssociatedTypeSpec {
    #[display("{}::Containment", _0)]
    JsonContainment(TVar),
    #[display("{}::FieldAccess", _0)]
    JsonFieldAccess(TVar),
}

impl AssociatedTypeSpec {
    pub(crate) fn depends_on(&self) -> &TVar {
        match self {
            Self::JsonContainment(spec) => spec,
            Self::JsonFieldAccess(spec) => spec,
        }
    }
}

impl InitType for AssociatedTypeSpec {
    fn init_type(&self, env: &TypeEnv, unifier: &mut Unifier<'_>) -> Result<Arc<Type>, TypeError> {
        match self {
            AssociatedTypeSpec::JsonContainment(tvar) => {
                Ok(Arc::new(Type::Associated(AssociatedType::Json(
                    JsonQueryType::Containment(env.get(tvar)?.init_type(env, unifier)?),
                ))))
            }

            AssociatedTypeSpec::JsonFieldAccess(tvar) => {
                Ok(Arc::new(Type::Associated(AssociatedType::Json(
                    JsonQueryType::FieldAccess(env.get(tvar)?.init_type(env, unifier)?),
                ))))
            }
        }
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
    /// The generic args of this function - the generic args are local to this function definition.
    pub(crate) generic_args: Vec<TVar>,
    /// The argument types.
    pub(crate) args: Vec<TypeSpec>,
    /// The return type.
    pub(crate) ret: Box<TypeSpec>,
    /// The bounds ('where' clause).
    pub(crate) bounds: TypeSpecBounds,
}

impl GeneralizedFunctionSpec {
    pub(crate) fn init(
        &self,
        unifier: &mut Unifier<'_>,
        args: &[Arc<Type>],
        ret: Arc<Type>,
    ) -> Result<InstantiatedTypeEnv, TypeError> {
        let mut env = TypeEnv::new();

        for Bounded(tvar, traits) in &self.bounds.0 {
            let inner_tvar = TVar(format!("{}_inner", tvar));
            env.add(
                tvar,
                TypeSpec::Var(VarSpec {
                    tvar: inner_tvar,
                    bounds: *traits,
                }),
            )?;
        }

        let instantiated_env = env.instantiate(unifier)?;

        for (idx, arg) in args.iter().enumerate() {
            unifier.unify(
                arg.clone(),
                instantiated_env.get_type(&self.args[idx], &unifier)?,
            )?;
        }

        // unifier.unify(ret.clone(), instantiated_env.get_type(&self.ret, &unifier)?)?;

        Ok(instantiated_env)
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
