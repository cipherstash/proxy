use std::{collections::HashMap, fmt::Display, marker::PhantomData};

use sqltk::{AsNodeKey, NodeKey, Visitable};

use crate::inference::unifier::{Type, TypeVar};

use super::{Sequence, SequenceVal};

/// `TypeRegistry` maintains an association between `sqlparser` AST nodes and the node's inferred [`Type`].
pub struct TypeRegistry<'ast> {
    tvar_seq: Sequence<TypeVar>,
    tid_seq: Sequence<TID>,
    types: HashMap<TID, Type>,
    substitutions: HashMap<TypeVar, TID>,
    node_types: HashMap<NodeKey<'ast>, TID>,
    param_types: HashMap<&'ast String, TID>,
    _ast: PhantomData<&'ast ()>,
}

/// A type ID.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[allow(non_camel_case_types)]
pub struct TID {
    id: u32,
}

impl Display for TID {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("TID:{}", self.id))
    }
}

impl TID {
    pub const NATIVE: Self = TID { id: 0 };
    pub const EMPTY_PROJECTION: Self = TID { id: 1 };
}

impl From<SequenceVal<TID>> for TID {
    fn from(value: SequenceVal<TID>) -> Self {
        Self { id: value.value }
    }
}

impl Default for TypeRegistry<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'ast> TypeRegistry<'ast> {
    /// Creates a new, empty `TypeRegistry`.
    pub fn new() -> Self {
        let mut types = HashMap::new();
        types.insert(TID::NATIVE, Type::any_native());
        types.insert(TID::EMPTY_PROJECTION, Type::empty_projection());

        Self {
            tvar_seq: Sequence::new(),
            tid_seq: Sequence::new_starting_at(TID::EMPTY_PROJECTION.id + 1),
            types,
            substitutions: HashMap::new(),
            node_types: HashMap::new(),
            param_types: HashMap::new(),
            _ast: PhantomData,
        }
    }

    pub(crate) fn get_nodes_and_types<N: Visitable>(&self) -> Vec<(&'ast N, Type)> {
        self.node_types
            .iter()
            .filter_map(|(key, tid)| key.get_as::<N>().map(|n| (n, self.get_type_by_tid(*tid))))
            .collect()
    }

    pub(crate) fn get_type_by_tid(&self, tid: TID) -> Type {
        self.types[&tid].clone()
    }

    pub(crate) fn get_node_tid<N: AsNodeKey>(&self, node: &'ast N) -> TID {
        self.node_types.get(&node.as_node_key()).cloned().unwrap()
    }

    pub(crate) fn register(&mut self, ty: Type) -> TID {
        let tid = TID::from(self.tid_seq.next_value());
        self.types.insert(tid, ty);
        tid
    }

    pub(crate) fn exists_node_with_type<N: Visitable>(&self, needle: &Type) -> bool {
        self.first_matching_node_with_type::<N>(needle).is_some()
    }

    pub(crate) fn first_matching_node_with_type<N: Visitable>(
        &self,
        needle: &Type,
    ) -> Option<(&'ast N, TID, Type)> {
        self.node_types.iter().find_map(|(key, tid)| {
            let node = key.get_as::<N>()?;
            let ty = self.get_type_by_tid(*tid);
            if needle == &ty {
                Some((node, *tid, ty))
            } else {
                None
            }
        })
    }

    pub(crate) fn get_param(&self, param: &'ast String) -> Option<(TID, Type)> {
        let tid = *self.param_types.get(param)?;
        let ty = self.get_type_by_tid(tid);
        Some((tid, ty))
    }

    pub(crate) fn set_param(&mut self, param: &'ast String, ty: Type) {
        let tid = self.register(ty);
        self.param_types.insert(param, tid);
    }

    pub(crate) fn get_params(&self) -> HashMap<&'ast String, Type> {
        self.param_types
            .iter()
            .map(|(param, tid)| (*param, self.get_type_by_tid(*tid)))
            .collect()
    }

    pub(crate) fn node_types(&self) -> HashMap<NodeKey<'ast>, Type> {
        self.node_types
            .iter()
            .map(|(node, tid)| (*node, self.get_type_by_tid(*tid)))
            .collect()
    }

    pub(crate) fn get_substitution(&self, tvar: TypeVar) -> Option<TID> {
        self.substitutions.get(&tvar).copied()
    }

    pub(crate) fn substitute(&mut self, tvar: TypeVar, sub_tid: TID) {
        self.substitutions.insert(tvar, sub_tid);
    }

    /// Gets (and creates, if required) the [`Type`] associated with a node. If the node does not already have an
    /// associated `Type` then a [`Type::Var`] will be assigned to the node with a fresh [`TypeVar`].
    pub(crate) fn get_or_init_type<N: AsNodeKey>(&mut self, node: &'ast N) -> TID {
        match self.node_types.get(&node.as_node_key()) {
            Some(tid) => *tid,
            None => {
                let tid = self.fresh_tvar();
                self.node_types.insert(node.as_node_key(), tid);
                tid
            }
        }
    }

    pub(crate) fn fresh_tvar(&mut self) -> TID {
        let tvar = self.tvar_seq.next_value();
        self.register(Type::Var(tvar))
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
                    self.get_type_by_tid(*tid),
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
