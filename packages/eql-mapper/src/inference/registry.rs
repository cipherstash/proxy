use std::{collections::HashMap, marker::PhantomData, sync::Arc};

use sqltk::{AsNodeKey, NodeKey};

use crate::{inference::unifier::{Type, TypeVar}, Param, ParamError};

use super::Sequence;

/// `TypeRegistry` maintains an association between `sqlparser` AST nodes and the node's inferred [`Type`].
pub struct TypeRegistry<'ast> {
    tvar_seq: Sequence<TypeVar>,
    types: HashMap<TypeVar, Arc<Type>>,
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
            types: HashMap::new(),
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
        self.types.get(&tvar).cloned()
    }

    pub(crate) fn first_matching_node_with_type<N: AsNodeKey>(
        &self,
        needle: &Type,
    ) -> Option<(&'ast N, Arc<Type>)> {
        self.node_types.iter().find_map(|(key, ty)| {
            let node = key.get_as::<N>()?;
            if needle == &**ty {
                Some((node, Arc::clone(&ty)))
            } else {
                None
            }
        })
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

    pub(crate) fn substitute(&mut self, tvar: TypeVar, sub_ty: impl Into<Arc<Type>>) -> Arc<Type> {
        let sub_ty: Arc<_> = sub_ty.into();
        self.types.insert(tvar, sub_ty.clone());
        sub_ty
    }

    /// Gets (and creates, if required) the [`Type`] associated with a node. If the node does not already have an
    /// associated `Type` then a fresh [`Type::Var`] will be assigned.
    fn get_or_init_node_type<N: AsNodeKey>(&mut self, node: &'ast N) -> Arc<Type> {
        match self.node_types.get(&node.as_node_key()).cloned() {
            Some(ty) => ty,
            None => {
                let ty = Arc::new(Type::Var(self.fresh_tvar()));
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
        self.tvar_seq.next_value()
    }
}

pub(crate) mod test_util {
    use sqlparser::ast::{
        Delete, Expr, Function, FunctionArguments, Insert, Query, Select, SelectItem, SetExpr,
        Statement, Value, Values,
    };
    use sqltk::{AsNodeKey, Break, Visitable, Visitor};
    use std::{any::type_name, convert::Infallible, fmt::Debug, ops::ControlFlow};
    use tracing::Level;

    use super::TypeRegistry;

    use std::fmt::Display;

    impl TypeRegistry<'_> {
        /// Dumps the type information for a specific AST node to STDERR.
        ///
        /// Useful when debugging tests.
        pub(crate) fn dump_node<N: AsNodeKey + Display + AsNodeKey + Debug>(&self, node: &N) {
            let key = node.as_node_key();
            if let Some(ty) = self.node_types.get(&key) {
                let ty_name = type_name::<N>();

                let span = tracing::span!(
                    Level::TRACE,
                    "Node+Type",
                    ast_ty = ty_name,
                    node = %node,
                    ty = %ty,
                );

                let _guard = span.enter();
            };
        }

        /// Dumps the type information for all nodes visited so far to STDERR.
        ///
        /// Useful when debugging tests.
        pub(crate) fn dump_all_nodes<N: Visitable>(&self, root_node: &N) {
            struct FindNodeFromKeyVisitor<'a>(&'a TypeRegistry<'a>);

            impl<'ast> Visitor<'ast> for FindNodeFromKeyVisitor<'_> {
                type Error = Infallible;

                fn enter<N: Visitable>(
                    &mut self,
                    node: &'ast N,
                ) -> ControlFlow<Break<Self::Error>> {
                    if let Some(node) = node.downcast_ref::<Statement>() {
                        self.0.dump_node(node);
                    }

                    if let Some(node) = node.downcast_ref::<Query>() {
                        self.0.dump_node(node);
                    }

                    if let Some(node) = node.downcast_ref::<Insert>() {
                        self.0.dump_node(node);
                    }

                    if let Some(node) = node.downcast_ref::<Delete>() {
                        self.0.dump_node(node);
                    }

                    if let Some(node) = node.downcast_ref::<Expr>() {
                        self.0.dump_node(node);
                    }

                    if let Some(node) = node.downcast_ref::<SetExpr>() {
                        self.0.dump_node(node);
                    }

                    if let Some(node) = node.downcast_ref::<Select>() {
                        self.0.dump_node(node);
                    }

                    if let Some(node) = node.downcast_ref::<SelectItem>() {
                        self.0.dump_node(node);
                    }

                    if let Some(node) = node.downcast_ref::<Function>() {
                        self.0.dump_node(node);
                    }

                    if let Some(node) = node.downcast_ref::<FunctionArguments>() {
                        self.0.dump_node(node);
                    }

                    if let Some(node) = node.downcast_ref::<Values>() {
                        self.0.dump_node(node);
                    }

                    if let Some(node) = node.downcast_ref::<Value>() {
                        self.0.dump_node(node);
                    }

                    ControlFlow::Continue(())
                }
            }

            root_node.accept(&mut FindNodeFromKeyVisitor(self));
        }
    }
}
