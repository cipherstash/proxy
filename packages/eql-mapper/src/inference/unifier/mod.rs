use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

mod eql_traits;
mod instantiated_type_env;
mod resolve_type;
mod type_decl;
mod type_env;
mod types;
mod unify_types;

use crate::inference::TypeError;

pub use eql_traits::*;
pub(crate) use type_decl::*;

use unify_types::UnifyTypes;

use sqltk::AsNodeKey;
pub(crate) use types::*;

pub(crate) use type_env::*;
pub use types::{EqlTerm, EqlValue, NativeValue, TableColumn};

use super::TypeRegistry;
use tracing::{event, instrument, Level, Span};

/// Implements the type unification algorithm.
///
/// Type unification is the process of determining a type variable substitution that makes two type expressions
/// identical. It involves solving equations between types, by recursively comparing their structure and binding type
/// variables to concrete types or other variables.
#[derive(Debug)]
pub struct Unifier<'ast> {
    registry: Rc<RefCell<TypeRegistry<'ast>>>,
}

impl<'ast> Unifier<'ast> {
    /// Creates a new `Unifier`.
    pub fn new(registry: impl Into<Rc<RefCell<TypeRegistry<'ast>>>>) -> Self {
        Self {
            registry: registry.into(),
        }
    }

    pub(crate) fn fresh_tvar(&self) -> Arc<Type> {
        self.fresh_bounded_tvar(EqlTraits::none())
    }

    pub(crate) fn fresh_bounded_tvar(&self, bounds: EqlTraits) -> Arc<Type> {
        Type::Var(Var(self.registry.borrow_mut().fresh_tvar(), bounds)).into()
    }

    pub(crate) fn get_substitutions(&self) -> HashMap<TypeVar, Arc<Type>> {
        self.registry.borrow().get_substititions()
    }

    /// Looks up a previously registered [`Type`] by its [`TypeVar`].
    pub(crate) fn get_type(&self, tvar: TypeVar) -> Option<Arc<Type>> {
        self.registry.borrow().get_type(tvar)
    }

    pub(crate) fn get_node_type<N: AsNodeKey>(&self, node: &'ast N) -> Arc<Type> {
        let node_type = { self.registry.borrow_mut().get_node_type(node) };
        node_type.follow_tvars(self)
    }

    pub(crate) fn peek_node_type<N: AsNodeKey>(&self, node: &'ast N) -> Option<Arc<Type>> {
        self.registry.borrow_mut().peek_node_type(node)
    }

    pub(crate) fn get_param_type(&mut self, param: &'ast String) -> Arc<Type> {
        self.registry.borrow_mut().get_param_type(param)
    }

    /// [`sqltk::parser::ast::Value`] nodes with type `Type::Var(_)` after the inference phase is complete will be unified
    /// with [`NativeValue`].
    ///
    /// This can happen when a literal or param is never used in an expression that would constrain its type.
    ///
    /// In that case, it is safe to resolve its type as native because it cannot possibly be an EQL type, which are
    /// always correctly inferred.
    pub(crate) fn resolve_unresolved_value_nodes(&mut self) -> Result<(), TypeError> {
        let unresolved_value_nodes: Vec<_> = self
            .registry
            .borrow()
            .get_nodes_and_types::<sqltk::parser::ast::Value>()
            .into_iter()
            .map(|(node, ty)| (node, ty.follow_tvars(&*self)))
            .filter(|(_, ty)| matches!(&**ty, Type::Var(_)))
            .collect();

        for (_, ty) in unresolved_value_nodes {
            self.unify(ty, Type::native().into())?;
        }

        Ok(())
    }

    pub(crate) fn resolve_unresolved_associated_types(&mut self) -> Result<(), TypeError> {
        let unresolved_associated_types: Vec<_> = self
            .registry
            .borrow()
            .get_nodes_and_types::<sqltk::parser::ast::Value>()
            .into_iter()
            .map(|(node, ty)| (node, ty.follow_tvars(&*self)))
            .filter_map(|(node, ty)| {
                if let Type::Associated(associated) = &*ty {
                    Some((node, associated.clone()))
                } else {
                    None
                }
            })
            .collect();

        for (_node, associated_ty) in unresolved_associated_types {
            associated_ty.resolve_selector_target(self)?;
        }

        Ok(())
    }

    pub(crate) fn substitute(&mut self, tvar: TypeVar, sub_ty: Arc<Type>) -> Arc<Type> {
        event!(
            target: "eql-mapper::EVENT_SUBSTITUTE",
            Level::TRACE,
            tvar = %tvar,
            sub_ty = %sub_ty,
        );

        self.registry.borrow_mut().substitute(tvar, sub_ty)
    }

    /// Unifies two [`Type`]s or fails with a [`TypeError`].
    ///
    /// "Type Unification" is a fancy term for finding a set of type variable substitutions for multiple types
    /// that make those types equal, or else fail with a type error.
    ///
    /// Successful unification does not guarantee that the returned type will be fully resolved (i.e. it can contain
    /// dangling type variables).
    ///
    /// Returns `Ok(ty)` if successful, or `Err(TypeError)` on failure.
    #[instrument(
        target = "eql-mapper::UNIFY",
        skip(self),
        level = "trace",
        err(Debug),
        fields(
            lhs = %lhs,
            rhs = %rhs,
            return = tracing::field::Empty,
        )
    )]
    pub(crate) fn unify(&mut self, lhs: Arc<Type>, rhs: Arc<Type>) -> Result<Arc<Type>, TypeError> {
        let span = Span::current();

        let result = (|| {
            // Short-circuit the unification when lhs & rhs are equal.
            if lhs == rhs {
                Ok(lhs.clone())
            } else {
                match (&*lhs, &*rhs) {
                    (Type::Value(lhs_c), Type::Value(rhs_c)) => self.unify_types(lhs_c, rhs_c),

                    (Type::Var(var), Type::Value(value)) | (Type::Value(value), Type::Var(var)) => {
                        self.unify_types(value, var)
                    }

                    (Type::Var(lhs_v), Type::Var(rhs_v)) => self.unify_types(lhs_v, rhs_v),

                    (Type::Value(value), Type::Associated(associated_type))
                    | (Type::Associated(associated_type), Type::Value(value)) => {
                        self.unify_types(associated_type, value)
                    }

                    (Type::Var(var), Type::Associated(associated_type))
                    | (Type::Associated(associated_type), Type::Var(var)) => {
                        self.unify_types(associated_type, var)
                    }

                    (Type::Associated(lhs_assoc), Type::Associated(rhs_assoc)) => {
                        self.unify_types(lhs_assoc, rhs_assoc)
                    }
                }
            }
        })();

        if let Ok(ref val) = result {
            span.record("return", tracing::field::display(val));
        }

        result
    }

    /// Unifies a type with a type variable.
    ///
    /// Attempts to unify the type with whatever the type variable is pointing to.
    fn unify_with_type_var(
        &mut self,
        ty: Arc<Type>,
        tvar: TypeVar,
        tvar_bounds: &EqlTraits,
    ) -> Result<Arc<Type>, TypeError> {
        let unified = match self.get_type(tvar) {
            Some(sub_ty) => {
                self.satisfy_bounds(&sub_ty, tvar_bounds)?;
                self.unify(ty, sub_ty)?
            }
            None => {
                if let Type::Var(Var(_, ty_bounds)) = &*ty {
                    if ty_bounds != tvar_bounds {
                        self.fresh_bounded_tvar(tvar_bounds.union(ty_bounds))
                    } else {
                        ty.clone()
                    }
                } else {
                    ty.clone()
                }
            }
        };

        Ok(self.substitute(tvar, unified))
    }

    /// Prove that `ty` satisfies `bounds`.
    ///
    /// If `ty` is a [`Type::Var`] this test always passes.
    ///
    /// # Rules
    ///
    /// 1. Native types satisfy all possible bounds.
    /// 2. EQL types satisfy bounds that they implement.
    /// 3. Arrays satisfy all bounds of their element type.
    /// 4. Projections satisfy the intersection of the bounds of their columns.
    ///    a. However, empty projections satisfy all possible bounds.
    /// 5. Type variables satisfy all bounds that they carry.
    fn satisfy_bounds(&mut self, ty: &Type, bounds: &EqlTraits) -> Result<(), TypeError> {
        if let Type::Var(_) = ty {
            return Ok(());
        }

        if &bounds.intersection(&ty.effective_bounds()) == bounds {
            Ok(())
        } else {
            Err(TypeError::UnsatisfiedBounds(
                Arc::new(ty.clone()),
                bounds.difference(&ty.effective_bounds()),
            ))
        }
    }
}

pub(crate) mod test_util {
    use sqltk::parser::ast::{
        Delete, Expr, Function, FunctionArgExpr, Insert, Query, Select, SelectItem, SetExpr,
        Statement, Value, Values,
    };
    use sqltk::{AsNodeKey, Break, Visitable, Visitor};
    use std::{any::type_name, convert::Infallible, fmt::Debug, ops::ControlFlow};
    use tracing::{event, Level};

    use crate::unifier::Unifier;

    use std::fmt::Display;

    impl<'ast> super::Unifier<'ast> {
        pub(crate) fn dump_substitutions(&self) {
            for (tvar, ty) in self.get_substitutions().iter() {
                event!(
                    target: "eql-mapper::DUMP_SUB",
                    Level::TRACE,
                    sub = format!("{} => {}", tvar, ty)
                );
            }
        }

        /// Dumps the type information for a specific AST node to STDERR.
        ///
        /// Useful when debugging tests.
        pub(crate) fn dump_node<N: AsNodeKey + Display + Debug>(&self, node: &'ast N) {
            let root_ty = self.get_node_type(node).clone();
            let found_ty = root_ty.clone().follow_tvars(self);
            let ast_ty = type_name::<N>();

            event!(
                target: "eql-mapper::DUMP_NODE",
                Level::TRACE,
                ast_ty = ast_ty,
                node = %node,
                root_ty = %root_ty,
                found_ty = %found_ty
            );
        }

        /// Dumps the type information for all nodes visited so far to STDERR.
        ///
        /// Useful when debugging tests.
        pub(crate) fn dump_all_nodes<N: Visitable>(&self, root_node: &'ast N) {
            struct FindNodeFromKeyVisitor<'a, 'ast>(&'a Unifier<'ast>);

            impl<'ast> Visitor<'ast> for FindNodeFromKeyVisitor<'_, 'ast> {
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

                    if let Some(node) = node.downcast_ref::<FunctionArgExpr>() {
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

            let _ = root_node.accept(&mut FindNodeFromKeyVisitor(self));
        }
    }
}

#[cfg(test)]
mod test {
    use eql_mapper_macros::shallow_init_types;

    use crate::unifier::Unifier;
    use crate::unifier::{EqlTraits, InstantiateType};
    use crate::{DepMut, TypeRegistry};

    #[test]
    fn eq_native() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        shallow_init_types! {&mut unifier, {
            let lhs = Native;
            let rhs = Native;
        }};

        assert_eq!(unifier.unify(lhs.clone(), rhs), Ok(lhs));
    }

    #[test]
    fn constructor_with_var() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        shallow_init_types! { &mut unifier, {
            let lhs = Native;
            let rhs = T;
        }};

        assert_eq!(unifier.unify(lhs.clone(), rhs), Ok(lhs));
    }

    #[test]
    fn var_with_constructor() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        shallow_init_types! {&mut unifier, {
            let lhs = T;
            let rhs = Native;
            let expected = Native;
        }};

        let actual = unifier.unify(lhs, rhs).unwrap();
        assert_eq!(actual, expected);
    }

    #[test]
    fn projections_without_wildcards() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        shallow_init_types! {&mut unifier, {
            let lhs = {Native, T};
            let rhs = {U, Native};
            let expected = {Native, Native};
        }};

        let actual = unifier.unify(lhs, rhs).unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    #[ignore = "this scenario cannot happen during unification because wildcards will have been expanded before the projections are unified"]
    // Leaving this test here as a reminder in case the above assertion proves to be false.
    fn projections_with_wildcards() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        shallow_init_types! {&mut unifier, {
            let lhs = {Native, Native};
            // rhs is a single projection that contains a projection column that contains a projection with two
            // projection columns.  This is how wildcard expansions is represented at the type level.
            let rhs = {{Native, Native}};
            let expected = {Native, Native};
        }};

        let actual = unifier.unify(lhs, rhs).unwrap();

        assert_eq!(actual, expected);
    }

    #[test]
    fn type_var_bounds_are_unified() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        shallow_init_types! {&mut unifier, {
            let lhs = T;
            let rhs = U;
        }};

        let unified = unifier.unify(lhs, rhs).unwrap();
        assert_eq!(unified.effective_bounds(), EqlTraits::default());

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        shallow_init_types! {&mut unifier, {
            let lhs = T;
            let rhs = U: Eq;
            let expected = V: Eq;
        }};

        let actual = unifier.unify(lhs, rhs).unwrap();

        assert_eq!(actual.effective_bounds(), expected.effective_bounds());

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        shallow_init_types! {&mut unifier, {
            let lhs = T: Eq;
            let rhs = U;
            let expected = V: Eq;
        }};

        let actual = unifier.unify(lhs, rhs).unwrap();

        assert_eq!(actual.effective_bounds(), expected.effective_bounds());
    }
}
