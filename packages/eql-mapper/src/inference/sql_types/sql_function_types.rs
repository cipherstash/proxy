use std::sync::Arc;

use derive_more::derive::Display;
use sqltk::parser::ast::{Function, FunctionArg, FunctionArgExpr, FunctionArguments, Ident};
use vec1::Vec1;

use crate::{
    unifier::{Type, TypeArg, TypeEnv},
    SqlIdent, TypeError, TypeInferencer,
};

/// The identifier and type signature of a SQL function.
///
/// See [`SQL_FUNCTION_SIGNATURES`].
#[derive(Debug)]
pub(crate) enum SqlFunction {
    Explicit(&'static ExplicitSqlFunctionRule),
    Fallback,
}

#[derive(Debug)]
pub(crate) struct ExplicitSqlFunctionRule {
    #[allow(unused)]
    pub(crate) name: CompoundIdent,
    pub(crate) sig: FunctionSig,
    pub(crate) rewrite_rule: RewriteRule,
}

impl ExplicitSqlFunctionRule {
    pub(crate) fn new(name: CompoundIdent, sig: FunctionSig) -> Self {
        Self {
            rewrite_rule: {
                // The logic here is that if there are no bounds on the function then the correct operation of the
                // function does not depend on custom handling of EQL types and the built-in SQL function will work just
                // fine.  SQL's `count` function is an example of this.
                if name.0.first() == &SqlIdent(Ident::new("pg_catalog"))
                    && sig.type_env.iter().all(|(_, bounds)| bounds.len() == 0)
                {
                    RewriteRule::UseEqlSchema
                } else {
                    RewriteRule::Ignore
                }
            },
            name,
            sig,
        }
    }
}

#[derive(Debug)]
pub(crate) enum RewriteRule {
    Ignore,
    UseEqlSchema,
}

/// The type signature of a SQL functon (excluding its name).
#[derive(Debug)]
pub(crate) struct FunctionSig {
    args: Vec<TypeArg>,
    return_type: TypeArg,
    type_env: TypeEnv,
}

/// A function signature but filled in with fresh type variables that correspond with the [`TypeArg`] or each argument and
/// return type.
#[derive(Debug, Clone)]
struct InstantiatedSig {
    args: Vec<Arc<Type>>,
    return_type: Arc<Type>,
}

impl FunctionSig {
    pub(crate) fn new(args: Vec<TypeArg>, return_type: TypeArg, type_env: TypeEnv) -> Self {
        for arg in &args {
            assert!(type_env.contains_key(arg));
        }
        assert!(type_env.contains_key(&return_type));

        Self {
            args,
            return_type,
            type_env,
        }
    }

    /// Creates an [`InstantiatedSig`] from `self`, filling in the [`TypeArg`]s with fresh type variables.
    fn instantiate(&self, inferencer: &TypeInferencer<'_>) -> Result<InstantiatedSig, TypeError> {
        let env = self
            .type_env
            .instantiate(&mut inferencer.unifier.borrow_mut())?;

        Ok(InstantiatedSig {
            args: self
                .args
                .iter()
                .map(|type_arg| {
                    env.get(&type_arg)
                        .cloned()
                        .expect("TypeArg should be present in TypeEnv")
                })
                .collect(),

            return_type: env
                .get(&self.return_type)
                .cloned()
                .expect("TypeArg should be present in TypeEnv"),
        })
    }

    /// Applies the type constraints of the function to to the AST.
    fn apply_constraints<'ast>(
        &self,
        inferencer: &mut TypeInferencer<'ast>,
        function: &'ast Function,
    ) -> Result<(), TypeError> {
        let InstantiatedSig { args, return_type } = self.instantiate(inferencer)?;
        let fn_name = CompoundIdent::from(&function.name.0);

        inferencer.unify_node_with_type(function, return_type.clone())?;

        match &function.args {
            FunctionArguments::None => {
                if args.is_empty() {
                    Ok(())
                } else {
                    Err(TypeError::Conflict(format!(
                        "expected {} args to function {}; got 0",
                        self.args.len(),
                        fn_name
                    )))
                }
            }

            FunctionArguments::Subquery(query) => {
                if self.args.len() == 1 {
                    inferencer.unify_node_with_type(&**query, args[0].clone())?;
                    Ok(())
                } else {
                    Err(TypeError::Conflict(format!(
                        "expected {} args to function {}; got 0",
                        args.len(),
                        fn_name
                    )))
                }
            }

            FunctionArguments::List(list) => {
                for (sig_arg, fn_arg) in args.iter().zip(list.args.iter()) {
                    let farg_expr = get_function_arg_expr(fn_arg);
                    inferencer.unify_node_with_type(farg_expr, sig_arg.clone())?;
                }

                Ok(())
            }
        }
    }
}

fn get_function_arg_expr(fn_arg: &FunctionArg) -> &FunctionArgExpr {
    match fn_arg {
        FunctionArg::Named { arg, .. } => arg,
        FunctionArg::ExprNamed { arg, .. } => arg,
        FunctionArg::Unnamed(arg) => arg,
    }
}

impl SqlFunction {
    pub(crate) fn apply_constraints<'ast>(
        &self,
        inferencer: &mut TypeInferencer<'ast>,
        function: &'ast Function,
    ) -> Result<(), TypeError> {
        match self {
            SqlFunction::Explicit(rule) => rule.sig.apply_constraints(inferencer, function),
            SqlFunction::Fallback => {
                match &function.args {
                    FunctionArguments::None => {}
                    FunctionArguments::Subquery(query) => {
                        inferencer.unify_node_with_type(&**query, Arc::new(Type::native()))?;
                    }
                    FunctionArguments::List(list) => {
                        for arg in &list.args {
                            let farg_expr = match arg {
                                FunctionArg::Named { arg, .. } => arg,
                                FunctionArg::ExprNamed { arg, .. } => arg,
                                FunctionArg::Unnamed(arg) => arg,
                            };

                            inferencer.unify_node_with_type(farg_expr, Arc::new(Type::native()))?;
                        }
                    }
                }

                inferencer.unify_node_with_type(function, Arc::new(Type::native()))?;
                Ok(())
            }
        }
    }
}

#[derive(Debug, Eq, PartialEq, PartialOrd, Ord, Hash, Clone, Display)]
#[display("{}", _0.iter().map(SqlIdent::to_string).collect::<Vec<_>>().join("."))]
pub(crate) struct CompoundIdent(Vec1<SqlIdent<Ident>>);

impl From<&Vec<Ident>> for CompoundIdent {
    fn from(value: &Vec<Ident>) -> Self {
        let mut idents = Vec1::<SqlIdent<Ident>>::new(SqlIdent(value[0].clone()));
        idents.extend(value[1..].iter().cloned().map(SqlIdent));
        CompoundIdent(idents)
    }
}
