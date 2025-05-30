use std::{cell::RefCell, collections::HashMap, rc::Rc, sync::Arc};

mod bounds;
mod type_env;
mod types;
mod unify_types;

use crate::inference::TypeError;

pub use bounds::*;

use unify_types::UnifyTypes;

use sqltk::AsNodeKey;
pub(crate) use types::*;

pub(crate) use type_env::*;
pub use types::{EqlValue, NativeValue, TableColumn};

use super::TypeRegistry;
use tracing::{event, instrument, Level};

/// Implements the type unification algorithm.
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

    // pub(crate) fn substitute_all_tvars_pointing_to_target(
    //     &self,
    //     target: TypeVar,
    //     replacement: Arc<Type>,
    // ) -> Arc<Type> {
    //     self.registry
    //         .borrow_mut()
    //         .substitute_all_tvars_pointing_to_target(target, replacement)
    // }

    pub(crate) fn fresh_tvar(&self) -> Arc<Type> {
        self.fresh_bounded_tvar(Bounds::none())
    }

    pub(crate) fn fresh_bounded_tvar(&self, bounds: Bounds) -> Arc<Type> {
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
        ret(Display),
        err(Debug),
        fields(
            lhs = %lhs,
            rhs = %rhs,
        )
    )]
    pub(crate) fn unify(&mut self, lhs: Arc<Type>, rhs: Arc<Type>) -> Result<Arc<Type>, TypeError> {
        // Short-circuit the unification when lhs & rhs are equal.
        if lhs == rhs {
            Ok(lhs.clone())
        } else {
            match (&*lhs, &*rhs) {
                (Type::Constructor(lhs_c), Type::Constructor(rhs_c)) => {
                    Ok(self.unify_types(lhs_c, rhs_c)?)
                }

                (Type::Var(var), Type::Constructor(constructor))
                | (Type::Constructor(constructor), Type::Var(var)) => {
                    Ok(self.unify_types(constructor, var)?)
                }

                (Type::Var(lhs_v), Type::Var(rhs_v)) => Ok(self.unify_types(lhs_v, rhs_v)?),
            }
        }
    }

    /// Unifies a type with a type variable.
    ///
    /// Attempts to unify the type with whatever the type variable is pointing to.
    fn unify_with_type_var(
        &mut self,
        ty: Arc<Type>,
        tvar: TypeVar,
        tvar_bounds: &Bounds,
    ) -> Result<Arc<Type>, TypeError> {
        let unified = match self.get_type(tvar) {
            Some(sub_ty) => {
                self.satisfy_bounds(&*sub_ty, tvar_bounds)?;
                self.unify(ty, sub_ty)?
            }
            None => {
                if let Type::Var(Var(_, ty_bounds)) = &*ty {
                    if ty_bounds != tvar_bounds {
                        self.fresh_bounded_tvar(tvar_bounds.union(&ty_bounds))
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
    ///     a. However, empty projections satisfy all possible bounds.
    /// 5. Type variables satisfy all bounds that they carry.
    fn satisfy_bounds(&mut self, ty: &Type, bounds: &Bounds) -> Result<(), TypeError> {
        if let Type::Var(_) = &*ty {
            return Ok(());
        }

        if &bounds.intersection(&ty.effective_bounds()) == bounds {
            return Ok(());
        } else {
            dbg!(&bounds);
            dbg!(&ty.effective_bounds());
            dbg!(&bounds.intersection(&ty.effective_bounds()));
            Err(TypeError::UnsatisfiedBounds(
                ty.clone(),
                bounds.difference(&ty.effective_bounds()),
            ))
        }
    }
}

pub(crate) mod test_util {
    use sqltk::parser::ast::{
        Delete, Expr, Function, FunctionArguments, Insert, Query, Select, SelectItem, SetExpr,
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
        pub(crate) fn dump_node<N: AsNodeKey + Display + AsNodeKey + Debug>(&self, node: &'ast N) {
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

            let _ = root_node.accept(&mut FindNodeFromKeyVisitor(self));
        }
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use crate::unifier::{
        Bounds, EqlTrait, NativeValue, Projection, ProjectionColumn, Type, TypeVar, Var,
    };
    use crate::unifier::{ProjectionColumns, Unifier};
    use crate::{DepMut, TypeRegistry};

    #[test]
    fn eq_native() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        let lhs: Arc<Type> = Type::from(NativeValue(None)).into();
        let rhs: Arc<Type> = Type::from(NativeValue(None)).into();

        assert_eq!(unifier.unify(lhs.clone(), rhs), Ok(lhs));
    }

    #[test]
    fn constructor_with_var() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        let lhs: Arc<Type> = NativeValue(None).into();
        let rhs: Arc<Type> = Var(TypeVar(0), Bounds::None).into();

        assert_eq!(unifier.unify(lhs.clone(), rhs), Ok(lhs));
    }

    #[test]
    fn var_with_constructor() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        let lhs: Arc<Type> = Var(TypeVar(0), Bounds::None).into();
        let rhs: Arc<Type> = NativeValue(None).into();

        assert_eq!(unifier.unify(lhs, rhs.clone()), Ok(rhs));
    }

    #[test]
    fn projections_without_wildcards() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        let lhs: Arc<Type> = Projection::WithColumns(ProjectionColumns(vec![
            ProjectionColumn::new(NativeValue(None), None),
            ProjectionColumn::new(Var(TypeVar(0), Bounds::None), None),
        ]))
        .into();

        let rhs: Arc<Type> = Projection::WithColumns(ProjectionColumns(vec![
            ProjectionColumn::new(Var(TypeVar(1), Bounds::None), None),
            ProjectionColumn::new(NativeValue(None), None),
        ]))
        .into();

        let unified = unifier.unify(lhs, rhs).unwrap();

        assert_eq!(
            *unified,
            Projection::WithColumns(ProjectionColumns(vec![
                ProjectionColumn::new(NativeValue(None), None),
                ProjectionColumn::new(NativeValue(None), None),
            ]))
            .into()
        );
    }

    #[test]
    fn projections_with_wildcards() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        let lhs: Arc<Type> = Projection::WithColumns(ProjectionColumns(vec![
            ProjectionColumn::new(NativeValue(None), None),
            ProjectionColumn::new(NativeValue(None), None),
        ]))
        .into();

        let cols: Arc<Type> = Projection::WithColumns(ProjectionColumns(vec![
            ProjectionColumn::new(NativeValue(None), None),
            ProjectionColumn::new(NativeValue(None), None),
        ]))
        .into();

        // The RHS is a single projection that contains a projection column that contains a projection with two
        // projection columns.  This is how wildcard expansions is represented at the type level.
        let rhs: Arc<Type> =
            Projection::WithColumns(ProjectionColumns(vec![ProjectionColumn::new(cols, None)]))
                .into();

        let unified = unifier.unify(lhs, rhs).unwrap();

        assert_eq!(
            *unified,
            Type::from(Projection::WithColumns(ProjectionColumns(vec![
                ProjectionColumn::new(NativeValue(None), None),
                ProjectionColumn::new(NativeValue(None), None),
            ])))
        );
    }

    #[test]
    fn type_var_bounds_are_unified() {
        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));

        let lhs: Arc<Type> = Var(TypeVar(0), Bounds::None).into();
        let rhs: Arc<Type> = Var(TypeVar(1), Bounds::None).into();

        let unified = unifier.unify(lhs, rhs).unwrap();
        assert!(matches!(&*unified, Type::Var(Var(_, Bounds::None))));

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));
        let lhs: Arc<Type> = Var(TypeVar(0), Bounds::None).into();
        let rhs: Arc<Type> = Var(TypeVar(1), Bounds::from(EqlTrait::Eq)).into();

        if let Type::Var(Var(_, bounds)) = &*unifier.unify(lhs, rhs).unwrap() {
            assert_eq!(bounds, &Bounds::from(EqlTrait::Eq));
        }

        let mut unifier = Unifier::new(DepMut::new(TypeRegistry::new()));
        let lhs: Arc<Type> = Var(TypeVar(0), Bounds::from(EqlTrait::Eq)).into();
        let rhs: Arc<Type> = Var(TypeVar(1), Bounds::None).into();

        if let Type::Var(Var(_, bounds)) = &*unifier.unify(lhs, rhs).unwrap() {
            assert_eq!(bounds, &Bounds::from(EqlTrait::Eq));
        }
    }
}
