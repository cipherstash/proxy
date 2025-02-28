use std::{any::TypeId, cell::RefCell, collections::HashMap, rc::Rc};

use sqltk::Semantic;

use crate::inference::{Def, Type, TypeError, TypeVar};

/// `TypeRegistry` maintains an association between `sqlparser` AST nodes and the node's inferred [`Type`].
#[derive(Debug)]
pub struct TypeRegistry {
    node_types: HashMap<NodeKey, Rc<RefCell<Type>>>,
}

impl Default for TypeRegistry {
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
pub struct NodeKey {
    node_addr: usize,
    node_type: TypeId,
}

impl<N: Semantic> From<&N> for NodeKey {
    fn from(value: &N) -> Self {
        NodeKey::new(value)
    }
}

impl NodeKey {
    /// Creates a `NodeKey` from an AST node reference (a [`Semantic`] impl).
    pub fn new<N: Semantic>(node: &N) -> Self {
        Self {
            node_addr: node as *const N as usize,
            node_type: TypeId::of::<N>(),
        }
    }

    #[cfg(test)]
    pub(crate) fn new_relaxed<N: 'static>(node: &N) -> Self {
        Self {
            node_addr: node as *const N as usize,
            node_type: TypeId::of::<N>(),
        }
    }

    /// Creates a `Vec<NodeKey>` from a slice of AST node references.
    pub fn from_slice<N: Semantic>(nodes: &[N]) -> Vec<Self> {
        nodes.iter().map(|n| Self::new(n)).collect()
    }
}

impl TypeRegistry {
    /// Creates a new, empty `TypeRegistry`.
    pub fn new() -> Self {
        Self {
            node_types: HashMap::new(),
        }
    }

    /// Gets (and creates, if required) the [`Type`] associated with a node (which must be an AST node type that
    /// implements [`Semantic`]). If the node does not already have an associated `Type` then a
    /// `Type(Def::Var(TypeVar::Fresh))` will be associated with the node and returned.
    ///
    /// This method is idempotent and further calls will return the same type.
    pub fn get_type<N: Semantic>(&mut self, node: &N) -> Rc<RefCell<Type>> {
        let ty = Type::new(Def::Var(TypeVar::Fresh)).wrap();

        let ty = &*self
            .node_types
            .entry(NodeKey::new(node))
            .or_insert(ty.clone());

        ty.clone()
    }

    /// Tries to resolve all types in `self`.
    ///
    /// If successful, returns `Ok(HashMap<NodeKey, Rc<RefCell<Type>>>)` else `Err(TypeError)`.
    pub fn try_resolve_all_types(&self) -> Result<HashMap<NodeKey, Rc<RefCell<Type>>>, TypeError> {
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

    impl TypeRegistry {
        /// Dumps the type information for a specific AST node to STDERR.
        ///
        /// Useful when debugging tests.
        pub(crate) fn dump_node<N: Display + 'static>(&self, node: &N) {
            let key = NodeKey::new_relaxed(node);
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
            struct FindNodeFromKeyVisitor<'a>(&'a TypeRegistry);

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
