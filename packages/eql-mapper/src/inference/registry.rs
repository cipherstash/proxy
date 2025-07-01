use std::{collections::HashMap, fmt::Debug, marker::PhantomData, sync::Arc};

use sqltk::{AsNodeKey, NodeKey};
use tracing::{span, Level};

use crate::{
    inference::unifier::{Type, TypeVar},
    unifier::Unifier,
    Param, ParamError,
};

use super::{
    unifier::{EqlTraits, Var},
    Sequence,
};

/// `TypeRegistry` maintains an association between `sqltk_parser` AST nodes and the node's inferred [`Type`].
#[derive(Debug)]
pub struct TypeRegistry<'ast> {
    tvar_seq: Sequence<TypeVar>,
    substitutions: HashMap<TypeVar, Arc<Type>>,
    node_types: HashMap<NodeKey<'ast>, TypeVar>,
    param_types: HashMap<&'ast String, TypeVar>,
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

    pub(crate) fn get_nodes_and_types<N: AsNodeKey + Debug>(&self) -> Vec<(&'ast N, Arc<Type>)> {
        let result = self
            .node_types
            .iter()
            .filter_map(|(key, tvar)| {
                key.get_as::<N>().map(|n| {
                    (
                        n,
                        self.substitutions
                            .get(tvar)
                            .cloned()
                            .unwrap_or(Arc::new(Type::Var(Var(*tvar, EqlTraits::default())))),
                    )
                })
            })
            .collect();

        result
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
            .map(|(param, tvar)| {
                (
                    *param,
                    self.substitutions
                        .get(tvar)
                        .cloned()
                        .unwrap_or(Arc::new(Type::Var(Var(*tvar, EqlTraits::none())))),
                )
            })
            .collect()
    }

    // TODO: move this logic to EqlMapper?
    pub(crate) fn resolved_param_types(
        &self,
        unifier: &Unifier<'ast>,
    ) -> Result<Vec<(Param, Arc<Type>)>, ParamError> {
        let mut params = self
            .get_params()
            .iter()
            .map(|(p, ty)| Param::try_from(*p).map(|p| (p, ty.clone().follow_tvars(unifier))))
            .collect::<Result<Vec<_>, _>>()?;

        params.sort_by(|(a, _), (b, _)| a.cmp(b));

        Ok(params)
    }

    pub(crate) fn node_types(&self) -> HashMap<NodeKey<'ast>, Arc<Type>> {
        self.node_types
            .iter()
            .map(|(node, tvar)| {
                (
                    *node,
                    self.substitutions
                        .get(tvar)
                        .cloned()
                        .unwrap_or(Arc::new(Type::Var(Var(*tvar, EqlTraits::none())))),
                )
            })
            .collect()
    }

    pub(crate) fn get_node_type<N: AsNodeKey>(&mut self, node: &'ast N) -> Arc<Type> {
        self.get_or_init_node_type(node)
    }

    pub(crate) fn peek_node_type<N: AsNodeKey>(&self, node: &'ast N) -> Option<Arc<Type>> {
        self.node_types
            .get(&node.as_node_key())
            .cloned()
            .map(|tvar| {
                self.substitutions
                    .get(&tvar)
                    .cloned()
                    .unwrap_or(Arc::new(Type::Var(Var(tvar, EqlTraits::none()))))
            })
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
                self.node_types.insert(node.as_node_key(), tvar);
                Type::Var(Var(tvar, EqlTraits::none())).into()
            }
        }
    }

    /// Gets (and creates, if required) the [`Type`] associated with a param. If the param does not already have an
    /// associated `Type` then a fresh [`Type::Var`] will be assigned.
    fn get_or_init_param_type(&mut self, param: &'ast String) -> Arc<Type> {
        match self.param_types.get(&param).cloned() {
            Some(tvar) => Type::Var(Var(tvar, EqlTraits::none())).into(),
            None => {
                let tvar = self.fresh_tvar();
                self.param_types.insert(param, tvar);
                Type::Var(Var(tvar, EqlTraits::none())).into()
            }
        }
    }

    pub(crate) fn fresh_tvar(&mut self) -> TypeVar {
        self.tvar_seq.next_value()
    }
}
