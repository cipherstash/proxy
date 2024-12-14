use std::{cell::RefCell, collections::HashMap, marker::PhantomData, rc::Rc};

use sqltk::Semantic;

use crate::inference::{
    unifier::{Type, TypeVar},
    TypeError,
};

use super::{NodeKey, Status, TypeVarGenerator};

/// `TypeRegistry` maintains an association between `sqlparser` AST nodes and the node's inferred [`Type`].
#[derive(Debug)]
pub struct TypeRegistry<'ast> {
    tvar_gen: TypeVarGenerator,
    substitutions: HashMap<TypeVar, (Rc<RefCell<Type>>, Status)>,
    node_types: HashMap<NodeKey<'ast>, Rc<RefCell<Type>>>,
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

    pub(crate) fn get_substitution(&self, tvar: TypeVar) -> Option<(Type, Status)> {
        self.substitutions
            .get(&tvar)
            .map(|(ty, status)| ((*ty).borrow().clone(), *status))
    }

    pub(crate) fn substitute(&mut self, tvar: TypeVar, sub: Type) {
        let status = sub.status(self);

        self.substitutions
            .entry(tvar)
            .and_modify(|value| {
                *value.0.borrow_mut() = sub.clone();
            })
            .or_insert((Rc::new(RefCell::new(sub)), status));
    }

    pub(crate) fn set_node_type<N: Semantic>(&mut self, node: &'ast N, ty: Type) -> Type {
        match self.node_types.get(&NodeKey::new(node)) {
            Some(existing_ty) => {
                *existing_ty.borrow_mut() = ty;
                existing_ty.borrow().clone()
            }
            None => {
                let ty = Rc::new(RefCell::new(ty));
                self.node_types.insert(NodeKey::new(node), ty.clone());
                let ty = &*ty.borrow();
                ty.clone()
            }
        }
    }

    /// Gets (and creates, if required) the [`Type`] associated with a node (which must be an AST node type that
    /// implements [`Semantic`]). If the node does not already have an associated `Type` then a
    /// `Type(Def::Var(TypeVar::Fresh))` will be associated with the node and returned.
    ///
    /// This method is idempotent and further calls will return the same type.
    pub(crate) fn get_type<N: Semantic>(&mut self, node: &'ast N) -> Type {
        let tvar = self.fresh_tvar();
        let ty = self
            .node_types
            .entry(NodeKey::new(node))
            .or_insert(Rc::new(RefCell::new(tvar)));
        (*ty).borrow().clone()
    }

    pub(crate) fn get_type_by_node_key(&self, key: &NodeKey) -> Option<Type> {
        self.node_types.get(key).map(|ty| (*ty).borrow().clone())
    }

    /// Tries to resolve all types in `self`.
    ///
    /// If successful, returns `Ok(HashMap<NodeKey, Type>)` else `Err(TypeError)`.
    pub(crate) fn try_resolve_all_types(
        &mut self,
    ) -> Result<HashMap<NodeKey<'ast>, Type>, TypeError> {
        let node_types = self.node_types.clone();
        let node_types: HashMap<_, _> = node_types
            .iter()
            .map(|(node_key, ty)| {
                let ty = (*ty).borrow().clone();
                ty.try_resolve(self).map(|ty| (node_key.clone(), ty))
            })
            .collect::<Result<HashMap<_, _>, _>>()?;

        Ok(node_types)
    }

    pub(crate) fn fresh_tvar(&mut self) -> Type {
        Type::Var(TypeVar(self.tvar_gen.next_tvar()))
    }
}

#[cfg(test)]
pub(crate) mod test_util {
    use sqlparser::ast::{
        Delete, Expr, Function, FunctionArguments, Insert, Query, Select, SetExpr, Statement,
    };
    use sqltk::{Break, Visitable, Visitor};
    use std::{convert::Infallible, fmt::Debug, ops::ControlFlow};
    use tracing::info;

    use super::{NodeKey, TypeRegistry};

    use std::fmt::Display;

    impl TypeRegistry<'_> {
        /// Dumps the type information for a specific AST node to STDERR.
        ///
        /// Useful when debugging tests.
        pub(crate) fn dump_node<N: Display + Visitable + Debug>(&self, node: &N) {
            let key = NodeKey::new_from_visitable(node);
            if let Some(ty) = self.node_types.get(&key) {
                info!(
                    "TYPE<\nast: {}\nsyn: {}\nty: {}\n>",
                    std::any::type_name::<N>(),
                    node,
                    &*ty.borrow(),
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
