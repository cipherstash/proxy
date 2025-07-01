use std::sync::{Arc, LazyLock};

use sqltk::parser::ast::{
    Function, FunctionArg, FunctionArgExpr, FunctionArguments, Ident, ObjectNamePart,
};

use crate::{
    unifier::{FunctionDecl, Type, Unifier},
    TypeError, TypeInferencer,
};

/// Either explicit typing rules for a function that supports EQL, or a fallback where the typing rules force all
/// function argument types and the return type to be native.
#[derive(Debug)]
pub(crate) enum SqlFunction {
    Explicit(&'static FunctionDecl),
    Fallback,
}

static PG_CATALOG: LazyLock<ObjectNamePart> =
    LazyLock::new(|| ObjectNamePart::Identifier(Ident::new("pg_catalog")));

impl SqlFunction {
    pub(crate) fn should_rewrite(&self) -> bool {
        match self {
            SqlFunction::Explicit(function_spec) => function_spec.name.0[0] == *PG_CATALOG,
            SqlFunction::Fallback => false,
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
        let ret_type = inferencer.get_node_type(function);
        match self {
            SqlFunction::Explicit(rule) => {
                match &function.args {
                    FunctionArguments::None => {
                        rule.inner
                            .apply(&mut inferencer.unifier.borrow_mut(), &[], ret_type)?
                    }
                    FunctionArguments::Subquery(query) => {
                        let node_type = inferencer.get_node_type(&**query);
                        rule.inner.apply(
                            &mut inferencer.unifier.borrow_mut(),
                            &[node_type],
                            ret_type,
                        )?
                    }
                    FunctionArguments::List(list) => {
                        let args: Vec<Arc<Type>> = list
                            .args
                            .iter()
                            .map(|arg| inferencer.get_node_type(get_function_arg_expr(arg)))
                            .collect();
                        rule.inner
                            .apply(&mut inferencer.unifier.borrow_mut(), &args, ret_type)?
                    }
                };

                Ok(())
            }
            SqlFunction::Fallback => {
                match &function.args {
                    FunctionArguments::None => NativeFunction::new(0).apply_constraints(
                        &mut inferencer.unifier.borrow_mut(),
                        &[],
                        ret_type,
                    )?,
                    FunctionArguments::Subquery(query) => {
                        let query_type = &[inferencer.get_node_type(&**query)];
                        NativeFunction::new(1).apply_constraints(
                            &mut inferencer.unifier.borrow_mut(),
                            query_type,
                            ret_type,
                        )?
                    }
                    FunctionArguments::List(list) => {
                        let args: Vec<Arc<Type>> = list
                            .args
                            .iter()
                            .map(|arg| inferencer.get_node_type(get_function_arg_expr(arg)))
                            .collect();
                        NativeFunction::new(args.len() as u8).apply_constraints(
                            &mut inferencer.unifier.borrow_mut(),
                            &args,
                            ret_type,
                        )?
                    }
                };

                Ok(())
            }
        }
    }
}

pub(crate) struct NativeFunction {
    arg_count: u8,
}

impl NativeFunction {
    pub fn new(arg_count: u8) -> Self {
        Self { arg_count }
    }

    pub(crate) fn apply_constraints(
        &self,
        unifier: &mut Unifier<'_>,
        args: &[Arc<Type>],
        ret: Arc<Type>,
    ) -> Result<(), TypeError> {
        if args.len() != self.arg_count as usize {
            return Err(TypeError::Expected(format!(
                "expected {} function arguments but for {}",
                self.arg_count,
                args.len()
            )));
        }

        for arg in args.iter() {
            unifier.unify(arg.clone(), Type::native().into())?;
        }

        unifier.unify(ret.clone(), Type::native().into())?;

        Ok(())
    }
}
