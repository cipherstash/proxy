use std::{collections::HashMap, fmt::Display, sync::Arc};

use crate::{
    unifier::{TVar, Type},
    TypeError,
};

#[derive(Debug, Clone, Default)]
pub(crate) struct InstantiatedTypeEnv {
    types: HashMap<TVar, Arc<Type>>,
}

impl InstantiatedTypeEnv {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn add_type(&mut self, tvar: TVar, ty: Arc<Type>) -> Result<(), TypeError> {
        if self.types.insert(tvar.clone(), ty).is_none() {
            Ok(())
        } else {
            Err(TypeError::InternalError(format!(
                "named type {tvar} already initialised in {self}"
            )))
        }
    }

    pub(crate) fn get_type(&self, tvar: &TVar) -> Result<Arc<Type>, TypeError> {
        match self.types.get(tvar).cloned() {
            Some(ty) => Ok(ty),
            None => Err(TypeError::InternalError(format!(
                "type for tvar {tvar} not found"
            ))),
        }
    }
}

impl Display for InstantiatedTypeEnv {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("InstantiatedTypeEnv{ ")?;
        for (idx, (tvar, spec)) in self.types.iter().enumerate() {
            f.write_fmt(format_args!("{tvar} => {spec}"))?;
            if idx < self.types.len() - 1 {
                f.write_str(", ")?;
            }
        }
        f.write_str("}")
    }
}
