use std::{collections::HashMap, marker::PhantomData, sync::Arc};

use sqltk::{AsNodeKey, NodeKey};
use tracing::{span, Level};

use crate::{
    inference::unifier::{Type, TypeVar}, Param, ParamError
};

use super::Sequence;

/// `TypeRegistry` maintains an association between `sqlparser` AST nodes and the node's inferred [`Type`].
#[derive(Debug)]
pub struct TypeRegistry<'ast> {
    tvar_seq: Sequence<TypeVar>,
    substitutions: HashMap<TypeVar, Arc<Type>>,
    node_types: HashMap<NodeKey<'ast>, Arc<Type>>,
    param_types: HashMap<&'ast String, Arc<Type>>,
    _ast: PhantomData<&'ast ()>,
}

impl Default for TypeRegistry<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'ast> TypeRegistry<'ast> {
    /// Creates a new, empty `TypeRegistry`.
    pub fn new() -> Self {
        Self {
            tvar_seq: Sequence::new(),
            substitutions: HashMap::new(),
            node_types: HashMap::new(),
            param_types: HashMap::new(),
            _ast: PhantomData,
        }
    }

    pub(crate) fn get_nodes_and_types<N: AsNodeKey>(&self) -> Vec<(&'ast N, Arc<Type>)> {
        self.node_types
            .iter()
            .filter_map(|(key, ty)| key.get_as::<N>().map(|n| (n, ty.clone())))
            .collect()
    }

    pub(crate) fn get_type(&self, tvar: TypeVar) -> Option<Arc<Type>> {
        span!(Level::TRACE, "GET TYPE");
        self.substitutions.get(&tvar).cloned()
    }

    pub(crate) fn get_substititions(&self) -> HashMap<TypeVar, Arc<Type>> {
        self.substitutions.clone()
    }

    pub(crate) fn get_param_type(&mut self, param: &'ast String) -> Arc<Type> {
        self.get_or_init_param_type(param)
    }

    pub(crate) fn get_params(&self) -> HashMap<&'ast String, Arc<Type>> {
        self.param_types
            .iter()
            .map(|(param, ty)| (*param, Arc::clone(ty)))
            .collect()
    }

    // TODO: move this logic to EqlMapper?
    pub(crate) fn resolved_param_types(&self) -> Result<Vec<(Param, Arc<Type>)>, ParamError> {
        let mut params = self
            .get_params()
            .iter()
            .map(|(p, ty)| Param::try_from(*p).map(|p| (p, ty.clone())))
            .collect::<Result<Vec<_>, _>>()?;

        params.sort_by(|(a, _), (b, _)| a.cmp(b));

        Ok(params)
    }

    pub(crate) fn node_types(&self) -> HashMap<NodeKey<'ast>, Arc<Type>> {
        self.node_types
            .iter()
            .map(|(node, ty)| (*node, Arc::clone(ty)))
            .collect()
    }

    pub(crate) fn get_node_type<N: AsNodeKey>(&mut self, node: &'ast N) -> Arc<Type> {
        self.get_or_init_node_type(node)
    }

    pub(crate) fn peek_node_type<N: AsNodeKey>(&self, node: &'ast N) -> Option<Arc<Type>> {
        self.node_types.get(&node.as_node_key()).cloned()
    }

    pub(crate) fn substitute(&mut self, tvar: TypeVar, sub_ty: impl Into<Arc<Type>>) -> Arc<Type> {
        let sub_ty: Arc<_> = sub_ty.into();
        self.substitutions.insert(tvar, sub_ty.clone());
        sub_ty
    }

    /// Gets (and creates, if required) the [`Type`] associated with a node. If the node does not already have an
    /// associated `Type` then a fresh [`Type::Var`] will be assigned.
    fn get_or_init_node_type<N: AsNodeKey>(&mut self, node: &'ast N) -> Arc<Type> {
        match self.peek_node_type(node) {
            Some(ty) => ty,
            None => {
                let tvar = self.fresh_tvar();
                let ty = Arc::new(Type::Var(tvar));
                self.node_types.insert(node.as_node_key(), ty.clone());
                ty
            }
        }
    }

    /// Gets (and creates, if required) the [`Type`] associated with a param. If the param does not already have an
    /// associated `Type` then a fresh [`Type::Var`] will be assigned.
    fn get_or_init_param_type(&mut self, param: &'ast String) -> Arc<Type> {
        match self.param_types.get(&param).cloned() {
            Some(ty) => ty,
            None => {
                let ty = Arc::new(Type::Var(self.fresh_tvar()));
                self.param_types.insert(param, ty.clone());
                ty
            }
        }
    }

    pub(crate) fn fresh_tvar(&mut self) -> TypeVar {
        let next = self.tvar_seq.next_value();
        if next == TypeVar(3) {
            println!("WAT");
        }
        next
    }
}

