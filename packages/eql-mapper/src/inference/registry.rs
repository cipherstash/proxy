use std::{any::TypeId, cell::RefCell, collections::HashMap, marker::PhantomData, rc::Rc};

use sqltk::{Semantic, Visitable};

use crate::inference::{
    unifier::{Def, Type, TypeVar},
    TypeError,
};

/// `TypeRegistry` maintains an association between `sqlparser` AST nodes and the node's inferred [`Type`].
#[derive(Debug)]
pub struct TypeRegistry<'ast> {
    node_types: HashMap<NodeKey<'ast>, Rc<RefCell<Type>>>,
    _ast: PhantomData<&'ast ()>,
}

impl Default for TypeRegistry<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// Acts as the key type for a [`HashMap`] so that a [`Semantic`] value of any concrete type can be used as a key in
/// the same `HashMap`.
///
/// It works by capturing the address of an AST node in addition to its [`TypeId`]. Both are required to uniquely
/// identify a node because different node values can have the same address (e.g. the address of a struct and the
/// address of its first field will be equal but the struct and the field are different values).
#[derive(Debug, Hash, Eq, PartialEq, Clone)]
pub struct NodeKey<'ast> {
    node_addr: usize,
    node_type: TypeId,
    _ast: PhantomData<&'ast ()>,
}

impl<'ast, N: Semantic> From<&'ast N> for NodeKey<'ast> {
    fn from(value: &'ast N) -> Self {
        NodeKey::new(value)
    }
}

impl<'ast> NodeKey<'ast> {
    /// Creates a `NodeKey` from an AST node reference (a [`Semantic`] impl).
    ///
    /// This method prevents mistakes like accidental annotation of a `Box<Expr>` instead of on the `Expr`.
    pub fn new<N: Semantic>(node: &'ast N) -> Self {
        Self {
            node_addr: node as *const N as usize,
            node_type: TypeId::of::<N>(),
            _ast: PhantomData,
        }
    }

    pub(crate) fn new_from_visitable<N: Visitable>(node: &'ast N) -> Self {
        Self {
            node_addr: node as *const N as usize,
            node_type: TypeId::of::<N>(),
            _ast: PhantomData,
        }
    }

    /// Creates a `Vec<NodeKey>` from a slice of AST node references.
    pub fn from_slice<N: Semantic>(nodes: &'ast [N]) -> Vec<Self> {
        nodes.iter().map(|n| Self::new(n)).collect()
    }

    pub fn get_as<N: Visitable>(&self) -> Option<&'ast N> {
        if self.node_type == TypeId::of::<N>() {
            // SAFETY: we have verified that `N` is of the correct type to permit the cast and because 'ast outlives
            // `self` we know that the node has not been dropped.
            unsafe { (self.node_addr as *const N).as_ref() }
        } else {
            None
        }
    }
}

impl<'ast> TypeRegistry<'ast> {
    /// Creates a new, empty `TypeRegistry`.
    pub fn new() -> Self {
        Self {
            node_types: HashMap::new(),
            _ast: PhantomData,
        }
    }

    /// Gets (and creates, if required) the [`Type`] associated with a node (which must be an AST node type that
    /// implements [`Semantic`]). If the node does not already have an associated `Type` then a
    /// `Type(Def::Var(TypeVar::Fresh))` will be associated with the node and returned.
    ///
    /// This method is idempotent and further calls will return the same type.
    pub(crate) fn get_type<N: Semantic>(&mut self, node: &'ast N) -> Rc<RefCell<Type>> {
        let ty = Type::new(Def::Var(TypeVar::Fresh)).wrap();

        let ty = &*self
            .node_types
            .entry(NodeKey::new(node))
            .or_insert(ty.clone());

        ty.clone()
    }

    pub(crate) fn get_type_by_node_key(&self, key: &NodeKey) -> Option<Rc<RefCell<Type>>> {
        self.node_types.get(key).cloned()
    }

    /// Tries to resolve all types in `self`.
    ///
    /// If successful, returns `Ok(HashMap<NodeKey, Rc<RefCell<Type>>>)` else `Err(TypeError)`.
    pub(crate) fn try_resolve_all_types(
        &self,
    ) -> Result<HashMap<NodeKey<'ast>, Rc<RefCell<Type>>>, TypeError> {
        for ty in self.node_types.values() {
            ty.borrow_mut().try_resolve()?;
        }

        Ok(self.node_types.clone())
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use sqlparser::ast::{
        Delete, Expr, Function, FunctionArguments, Insert, Query, Select, SetExpr, Statement,
    };
    use sqltk::{Break, Visitable, Visitor};
    use std::{convert::Infallible, ops::ControlFlow};

    use super::{NodeKey, TypeRegistry};

    use std::fmt::Display;

    impl TypeRegistry<'_> {
        /// Dumps the type information for a specific AST node to STDERR.
        ///
        /// Useful when debugging tests.
        pub(crate) fn dump_node<N: Display + Visitable>(&self, node: &N) {
            let key = NodeKey::new_from_visitable(node);
            if let Some(ty) = self.node_types.get(&key) {
                eprintln!(
                    "{}\n   {}\n   {}\n\n",
                    std::any::type_name::<N>(),
                    node,
                    *ty.borrow()
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

                    if let Some(node) = node.downcast_ref::<Function>() {
                        self.0.dump_node(node);
                    }

                    if let Some(node) = node.downcast_ref::<FunctionArguments>() {
                        self.0.dump_node(node);
                    }

                    ControlFlow::Continue(())
                }
            }

            root_node.accept(&mut FindNodeFromKeyVisitor(self));
        }
    }
}
