use std::{any::TypeId, marker::PhantomData};

use sqltk::{Semantic, Visitable};

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
