use std::{
    collections::{HashMap, HashSet},
    sync::{Arc, LazyLock},
};

use derive_more::derive::Display;
use sqltk::parser::ast::{Function, FunctionArg, FunctionArgExpr, FunctionArguments, Ident};

use itertools::Itertools;
use vec1::{vec1, Vec1};

use crate::{sql_fn, unifier::Type, SqlIdent, TypeInferencer};

use super::TypeError;

#[derive(Debug)]
pub(crate) struct SqlFunction(CompoundIdent, FunctionSig);

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub(crate) enum Kind {
    Native,
    Generic(&'static str),
}

#[derive(Debug, Clone)]
pub(crate) struct FunctionSig {
    args: Vec<Kind>,
    return_type: Kind,
    generics: HashSet<&'static str>,
}

#[derive(Debug, Clone)]
pub(crate) struct InstantiatedSig {
    args: Vec<Arc<Type>>,
    return_type: Arc<Type>,
}

impl FunctionSig {
    fn new(args: Vec<Kind>, return_type: Kind) -> Self {
        let mut generics: HashSet<&'static str> = HashSet::new();

        for arg in &args {
            if let Kind::Generic(generic) = arg {
                generics.insert(*generic);
            }
        }

        if let Kind::Generic(generic) = return_type {
            generics.insert(generic);
        }

        Self {
            args,
            return_type,
            generics,
        }
    }

    pub(crate) fn is_applicable_to_args(&self, fn_args_syntax: &FunctionArguments) -> bool {
        match fn_args_syntax {
            FunctionArguments::None => self.args.is_empty(),
            FunctionArguments::Subquery(_) => self.args.len() == 1,
            FunctionArguments::List(fn_args) => self.args.len() == fn_args.args.len(),
        }
    }

    pub(crate) fn instantiate(&self, inferencer: &TypeInferencer<'_>) -> InstantiatedSig {
        let mut generics: HashMap<&'static str, Arc<Type>> = HashMap::new();

        for generic in self.generics.iter() {
            generics.insert(generic, inferencer.fresh_tvar());
        }

        InstantiatedSig {
            args: self
                .args
                .iter()
                .map(|kind| match kind {
                    Kind::Native => Arc::new(Type::any_native()),
                    Kind::Generic(generic) => generics[generic].clone(),
                })
                .collect(),

            return_type: match self.return_type {
                Kind::Native => Arc::new(Type::any_native()),
                Kind::Generic(generic) => generics[generic].clone(),
            },
        }
    }

    pub(crate) fn instantiate_native(function: &Function) -> InstantiatedSig {
        let arg_count = match &function.args {
            FunctionArguments::None => 0,
            FunctionArguments::Subquery(_) => 1,
            FunctionArguments::List(args) => args.args.len(),
        };

        let args: Vec<Arc<Type>> = (0..arg_count)
            .into_iter()
            .map(|_| Arc::new(Type::any_native()))
            .collect();

        InstantiatedSig {
            args,
            return_type: Arc::new(Type::any_native()),
        }
    }
}

impl InstantiatedSig {
    pub(crate) fn apply_constraints<'ast>(
        &self,
        inferencer: &mut TypeInferencer<'ast>,
        function: &'ast Function,
    ) -> Result<(), TypeError> {
        let fn_name = CompoundIdent::from(&function.name.0);

        // let function_ty = inferencer.get_node_type(function);

        inferencer.unify_node_with_type(function, self.return_type.clone())?;

        match &function.args {
            FunctionArguments::None => {
                if self.args.len() == 0 {
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
                    inferencer.unify_node_with_type(&**query, self.args[0].clone())?;
                    Ok(())
                } else {
                    Err(TypeError::Conflict(format!(
                        "expected {} args to function {}; got 0",
                        self.args.len(),
                        fn_name
                    )))
                }
            }

            FunctionArguments::List(args) => {
                for (sig_arg, fn_arg) in self.args.iter().zip(args.args.iter()) {
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
    fn new(ident: &str, sig: FunctionSig) -> Self {
        Self(CompoundIdent::from(ident), sig)
    }
}

#[derive(Debug, Eq, PartialEq, PartialOrd, Ord, Hash, Clone, Display)]
#[display("{}", _0.iter().map(SqlIdent::to_string).collect::<Vec<_>>().join("."))]
pub(crate) struct CompoundIdent(Vec1<SqlIdent<Ident>>);

impl From<&str> for CompoundIdent {
    fn from(value: &str) -> Self {
        CompoundIdent(vec1![SqlIdent(Ident::new(value))])
    }
}

impl From<&Vec<Ident>> for CompoundIdent {
    fn from(value: &Vec<Ident>) -> Self {
        let mut idents = Vec1::<SqlIdent<Ident>>::new(SqlIdent(value[0].clone()));
        idents.extend(value[1..].into_iter().cloned().map(SqlIdent));
        CompoundIdent(idents)
    }
}

static SQL_FUNCTION_SIGNATURES: LazyLock<HashMap<CompoundIdent, Vec<FunctionSig>>> = LazyLock::new(|| {
    // Notation: a single uppercase letter denotes an unknown type. Matching letters in a signature will be assigned
    // *the same type variable* and thus must resolve to the same type. (🙏 Haskell)
    //
    // Eventually we should type check EQL types against their configured indexes instead of leaving that to the EQL
    // extension in the database. I can imagine supporting type bounds in signatures here, such as: `T: Eq`
    let sql_fns = vec![
        sql_fn!(count(T) -> NATIVE),
        sql_fn!(min(T) -> T),
        sql_fn!(max(T) -> T),
        sql_fn!(jsonb_path_query(T, T) -> T),
    ];

    let mut sql_fns_by_name: HashMap<CompoundIdent, Vec<FunctionSig>> = HashMap::new();

    for (key, chunk) in &sql_fns.into_iter().chunk_by(|sql_fn| sql_fn.0.clone()) {
        sql_fns_by_name.insert(
            key.clone(),
            chunk.into_iter().map(|sql_fn| sql_fn.1).collect(),
        );
    }

    sql_fns_by_name
});

pub(crate) fn get_type_signature_for_special_cased_sql_function(
    fn_name: &CompoundIdent,
    args: &FunctionArguments,
) -> Option<&'static FunctionSig> {
    let sigs = SQL_FUNCTION_SIGNATURES.get(fn_name)?;
    sigs.iter().find(|sig| sig.is_applicable_to_args(args))
}
