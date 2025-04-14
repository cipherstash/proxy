use std::{collections::HashMap, marker::PhantomData};

use sqlparser::ast::Expr;
use sqltk::{AsNodeKey, NodeKey};

use crate::inference::unifier::{Type, TypeVar};

use super::{TypeCell, TypeVarGenerator};

/// `TypeRegistry` maintains an association between `sqlparser` AST nodes and the node's inferred [`Type`].
#[derive(Debug)]
pub struct TypeRegistry<'ast> {
    tvar_gen: TypeVarGenerator,
    substitutions: HashMap<TypeVar, TypeCell>,
    node_types: HashMap<NodeKey<'ast>, TypeCell>,
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
            tvar_gen: TypeVarGenerator::new(),
            substitutions: HashMap::new(),
            node_types: HashMap::new(),
            _ast: PhantomData,
        }
    }

    pub(crate) fn get_substitution(&self, tvar: TypeVar) -> Option<TypeCell> {
        self.substitutions.get(&tvar).cloned()
    }

    pub(crate) fn substitute(&mut self, tvar: TypeVar, sub: TypeCell) {
        self.substitutions.insert(tvar, sub);
    }

    /// Gets the [`Type`] associated with a node.
    pub(crate) fn get_type<N: AsNodeKey>(&self, node: &'ast N) -> Option<&TypeCell> {
        self.node_types.get(&node.as_node_key())
    }

    /// Gets (and creates, if required) the [`Type`] associated with a node. If the node does not already have an
    /// associated `Type` then a [`Type::Var`] will be assigned to the node with a fresh [`TypeVar`].
    pub(crate) fn get_or_init_type<N: AsNodeKey>(&mut self, node: &'ast N) -> &TypeCell {
        let tvar = self.fresh_tvar();

        self.node_types.entry(node.as_node_key()).or_insert(tvar)
    }

    pub(crate) fn get_type_by_node_key(&self, key: &NodeKey<'ast>) -> Option<&TypeCell> {
        match self.node_types.get(key) {
            Some(ty) => Some(ty),
            None => None,
        }
    }

    pub(crate) fn fresh_tvar(&mut self) -> TypeCell {
        Type::Var(TypeVar(self.tvar_gen.next_tvar())).into_type_cell()
    }

    /// Checks if any node in the AST that has type `ty` is a literal.
    pub(crate) fn value_expr_exists_with_type(&self, ty: TypeCell) -> bool {
        for (node_key, node_ty) in self.node_types.iter() {
            if ty == *node_ty {
                if let Some(Expr::Value(_)) = node_key.get_as::<Expr>() {
                    return true;
                }
            }
        }

        false
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use sqlparser::ast::{
        Delete, Expr, Function, FunctionArguments, Insert, Query, Select, SelectItem, SetExpr,
        Statement,
    };
    use sqltk::{AsNodeKey, Break, Visitable, Visitor};
    use std::{convert::Infallible, fmt::Debug, ops::ControlFlow};
    use tracing::error;

    use super::TypeRegistry;

    use std::fmt::Display;

    impl TypeRegistry<'_> {
        /// Dumps the type information for a specific AST node to STDERR.
        ///
        /// Useful when debugging tests.
        pub(crate) fn dump_node<N: Display + AsNodeKey + Debug>(&self, node: &N) {
            let key = node.as_node_key();
            if let Some(ty) = self.node_types.get(&key) {
                error!(
                    "TYPE<\nast: {}\nsyn: {}\nty: {}\n>",
                    std::any::type_name::<N>(),
                    node,
                    &*ty.as_type(),
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

                    ControlFlow::Continue(())
                }
            }

            root_node.accept(&mut FindNodeFromKeyVisitor(self));
        }
    }
}
