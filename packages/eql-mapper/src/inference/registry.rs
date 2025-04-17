use std::{collections::HashMap, marker::PhantomData, sync::Arc};

use sqltk::{AsNodeKey, NodeKey, Visitable};

use crate::inference::unifier::{Type, TypeVar};

use super::Sequence;

/// `TypeRegistry` maintains an association between `sqlparser` AST nodes and the node's inferred [`Type`].
pub struct TypeRegistry<'ast> {
    tvar_seq: Sequence<TypeVar>,
    types: HashMap<TypeVar, Arc<Type>>,
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
            types: HashMap::new(),
            node_types: HashMap::new(),
            param_types: HashMap::new(),
            _ast: PhantomData,
        }
    }

    pub(crate) fn get_nodes_and_types<N: Visitable>(&self) -> Vec<(&'ast N, Option<Arc<Type>>)> {
        self.node_types
            .iter()
            .filter_map(|(key, tid)| key.get_as::<N>().map(|n| (n, self.get_type_by_tvar(*tid))))
            .collect()
    }

    pub(crate) fn get_type_by_tvar(&self, tvar: TypeVar) -> Option<Arc<Type>> {
        self.types.get(&tvar).cloned()
    }

    pub(crate) fn register(&mut self, ty: impl Into<Arc<Type>>) -> TypeVar {
        let tvar = self.fresh_tvar();
        self.types.insert(tvar, ty.into());
        tvar
    }

    pub(crate) fn exists_node_with_type<N: Visitable>(&self, needle: &Type) -> bool {
        self.first_matching_node_with_type::<N>(needle).is_some()
    }

    pub(crate) fn first_matching_node_with_type<N: Visitable>(
        &self,
        needle: &Type,
    ) -> Option<(&'ast N, TypeVar, Arc<Type>)> {
        self.node_types.iter().find_map(|(key, tvar)| {
            let node = key.get_as::<N>()?;
            if let Some(ty) = self.get_type_by_tvar(*tvar) {
                if needle == &*ty {
                    Some((node, *tvar, ty))
                } else {
                    None
                }
            } else {
                None
            }
        })
    }

    pub(crate) fn get_param(&self, param: &'ast String) -> Option<Arc<Type>> {
        let tvar = *self.param_types.get(param)?;
        self.get_type_by_tvar(tvar)
    }

    pub(crate) fn set_param(&mut self, param: &'ast String, ty: impl Into<Arc<Type>>) {
        let tid = self.register(ty);
        self.param_types.insert(param, tid);
    }

    pub(crate) fn get_params(&self) -> HashMap<&'ast String, Arc<Type>> {
        self.param_types
            .iter()
            .map(|(param, tvar)| (*param, Type::Var(*tvar).into()))
            .collect()
    }

    pub(crate) fn node_types(&self) -> HashMap<NodeKey<'ast>, Option<Arc<Type>>> {
        self.node_types
            .iter()
            .map(|(node, tvar)| {
                (
                    *node,
                    self.get_type_by_tvar(*tvar)
                )
            })
            .collect()
    }

    pub(crate) fn get_node_type<N: AsNodeKey>(&mut self, node: &'ast N) -> Arc<Type> {
        let tvar = self.get_or_init_type(node);
        self.get_type_by_tvar(tvar).unwrap()
    }

    pub(crate) fn get_substitution(&self, tvar: TypeVar) -> Option<Arc<Type>> {
        self.types.get(&tvar).cloned()
    }

    pub(crate) fn substitute(&mut self, tvar: TypeVar, sub_ty: impl Into<Arc<Type>>) -> Arc<Type> {
        let sub_ty: Arc<_> = sub_ty.into();
        self.types.insert(tvar, sub_ty.clone());
        sub_ty
    }

    /// Gets (and creates, if required) the [`Type`] associated with a node. If the node does not already have an
    /// associated `Type` then a [`Type::Var`] will be assigned to the node with a fresh [`TypeVar`].
    pub(crate) fn get_or_init_type<N: AsNodeKey>(&mut self, node: &'ast N) -> TypeVar {
        match self.node_types.get(&node.as_node_key()) {
            Some(tvar) => *tvar,
            None => {
                let node_tvar = self.fresh_tvar();
                self.node_types.insert(node.as_node_key(), node_tvar);
                let dangling_tvar = self.fresh_tvar();
                self.types.insert(node_tvar, Type::Var(dangling_tvar).into());
                node_tvar
            }
        }
    }

    pub(crate) fn fresh_tvar(&mut self) -> TypeVar {
        self.tvar_seq.next_value()
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use sqlparser::ast::{
        Delete, Expr, Function, FunctionArguments, Insert, Query, Select, SelectItem, SetExpr,
        Statement, Value,
    };
    use sqltk::{AsNodeKey, Break, Visitable, Visitor};
    use std::{convert::Infallible, fmt::Debug, ops::ControlFlow};

    use super::TypeRegistry;

    use std::fmt::Display;

    impl TypeRegistry<'_> {
        /// Dumps the type information for a specific AST node to STDERR.
        ///
        /// Useful when debugging tests.
        pub(crate) fn dump_node<N: Visitable + Display + AsNodeKey + Debug>(&self, node: &N) {
            let key = node.as_node_key();
            if let Some(tid) = self.node_types.get(&key) {
                tracing::error!(
                    "TYPE<\nast: {}\nsyn: {}\nty: {}\n>",
                    std::any::type_name::<N>(),
                    node,
                    self.get_type_by_tvar(*tid)
                        .map(|ty| ty.to_string())
                        .unwrap_or(String::from("<unresolved>")),
                );
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
