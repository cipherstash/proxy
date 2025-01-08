use std::{collections::HashMap, marker::PhantomData};

use sqltk::Semantic;

use crate::inference::unifier::{Type, TypeVar};

use super::{NodeKey, TypeCell, TypeVarGenerator};

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

    /// Gets (and creates, if required) the [`Type`] associated with a node (which must be an AST node type that
    /// implements [`Semantic`]). If the node does not already have an associated `Type` then a
    /// `Type(Def::Var(TypeVar::Fresh))` will be associated with the node and returned.
    ///
    /// This method is idempotent and further calls will return the same type.
    pub(crate) fn get_type<N: Semantic>(&mut self, node: &'ast N) -> &TypeCell {
        let tvar = self.fresh_tvar();

        self.node_types.entry(NodeKey::new(node)).or_insert(tvar)
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
}

#[cfg(test)]
pub(crate) mod test_util {
    use sqlparser::ast::{
        Delete, Expr, Function, FunctionArguments, Insert, Query, Select, SelectItem, SetExpr,
        Statement,
    };
    use sqltk::{Break, Visitable, Visitor};
    use std::{convert::Infallible, fmt::Debug, ops::ControlFlow};

    use super::{NodeKey, TypeRegistry};

    use std::fmt::Display;

    impl TypeRegistry<'_> {
        /// Dumps the type information for a specific AST node to STDERR.
        ///
        /// Useful when debugging tests.
        pub(crate) fn dump_node<N: Display + Visitable + Debug>(&self, node: &N) {
            let key = NodeKey::new_from_visitable(node);
            if let Some(ty) = self.node_types.get(&key) {
                eprintln!(
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
