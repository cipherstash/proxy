//! The [`UnifyTypes`] trait definition and all of the implementations.
//!
//! The entry point for [`Type`] unification is [`Unifier::unify`] which is an inherent method on the [`Unifier`] itself
//! and not part of the `UnifyTypes` trait.

use std::sync::Arc;

use crate::{unifier::SetOf, TypeError};

use super::{
    Array, AssociatedType, EqlTerm, NativeValue, Projection, ProjectionColumn, Type, Unifier,
    Value, Var,
};

/// Trait for unifying two types.
///
/// The `Lhs` and `Rhs` type arguments are independenty specifiable because some different base types (such as `Var` +
/// `Constructor` and `Value` + `Projection`) can be unified.
pub(super) trait UnifyTypes<Lhs, Rhs> {
    /// Try to unify types `lhs` & `rhs` to produce a new [`Type`].
    fn unify_types(&mut self, lhs: &Lhs, rhs: &Rhs) -> Result<Arc<Type>, TypeError>;
}

impl UnifyTypes<SetOf, SetOf> for Unifier<'_> {
    fn unify_types(&mut self, lhs: &SetOf, rhs: &SetOf) -> Result<Arc<Type>, TypeError> {
        Ok(Type::set_of(self.unify(lhs.inner_ty(), rhs.inner_ty())?).into())
    }
}

// A Value can be unified with a single-column Projection.
impl UnifyTypes<Value, Projection> for Unifier<'_> {
    fn unify_types(&mut self, lhs: &Value, rhs: &Projection) -> Result<Arc<Type>, TypeError> {
        let len = rhs.len();
        if len == 1 {
            self.unify_types(lhs, &rhs[0].ty)
        } else {
            Err(TypeError::Conflict(format!(
                "cannot unify value type {} with projection with > 1 column (it has {} columns) {}",
                lhs, len, rhs
            )))
        }
    }
}

impl UnifyTypes<Value, Arc<Type>> for Unifier<'_> {
    fn unify_types(&mut self, lhs: &Value, rhs: &Arc<Type>) -> Result<Arc<Type>, TypeError> {
        self.unify(lhs.clone().into(), rhs.clone())
    }
}

impl UnifyTypes<Value, Value> for Unifier<'_> {
    fn unify_types(&mut self, lhs: &Value, rhs: &Value) -> Result<Arc<Type>, TypeError> {
        match (lhs, rhs) {
            (Value::Eql(lhs), Value::Eql(rhs)) => self.unify_types(lhs, rhs),

            (Value::Native(lhs), Value::Native(rhs)) => self.unify_types(lhs, rhs),

            (Value::Array(lhs), Value::Array(rhs)) => self.unify_types(lhs, rhs),

            (Value::Projection(lhs), Value::Projection(rhs)) => self.unify_types(lhs, rhs),

            (Value::SetOf(lhs), Value::SetOf(rhs)) => self.unify_types(lhs, rhs),

            // Special case: a value can be unified with a single-column projection (producing a value).
            (value, Value::Projection(projection)) | (Value::Projection(projection), value) => {
                self.unify_types(value, projection)
            }

            (lhs, rhs) => Err(TypeError::Conflict(format!(
                "cannot unify values {} and {}",
                lhs, rhs
            ))),
        }
    }
}

impl UnifyTypes<Array, Array> for Unifier<'_> {
    fn unify_types(&mut self, lhs: &Array, rhs: &Array) -> Result<Arc<Type>, TypeError> {
        let Array(lhs_element_ty) = lhs;
        let Array(rhs_element_ty) = rhs;

        self.unify(lhs_element_ty.clone(), rhs_element_ty.clone())
    }
}

impl UnifyTypes<EqlTerm, EqlTerm> for Unifier<'_> {
    fn unify_types(&mut self, lhs: &EqlTerm, rhs: &EqlTerm) -> Result<Arc<Type>, TypeError> {
        match (lhs, rhs) {
            (EqlTerm::Full(lhs), EqlTerm::Full(rhs)) if lhs == rhs => {
                Ok(EqlTerm::Full(lhs.clone()).into())
            }

            (EqlTerm::Partial(lhs_eql, lhs_bounds), EqlTerm::Partial(rhs_eql, rhs_bounds))
                if lhs_eql == rhs_eql =>
            {
                Ok(EqlTerm::Partial(lhs_eql.clone(), lhs_bounds.union(rhs_bounds)).into())
            }

            (EqlTerm::Full(full), EqlTerm::Partial(partial, _))
            | (EqlTerm::Partial(partial, _), EqlTerm::Full(full))
                if full == partial =>
            {
                Ok(EqlTerm::Full(full.clone()).into())
            }

            (EqlTerm::JsonAccessor(lhs), EqlTerm::JsonAccessor(rhs)) if lhs == rhs => {
                Ok(EqlTerm::JsonAccessor(lhs.clone()).into())
            }

            (EqlTerm::JsonPath(lhs), EqlTerm::JsonPath(rhs)) if lhs == rhs => {
                Ok(EqlTerm::JsonPath(lhs.clone()).into())
            }

            (EqlTerm::Tokenized(lhs), EqlTerm::Tokenized(rhs)) if lhs == rhs => {
                Ok(EqlTerm::Tokenized(lhs.clone()).into())
            }

            (_, _) => Err(TypeError::Conflict(format!(
                "cannot unify EQL terms {} and {}",
                lhs, rhs
            ))),
        }
    }
}

impl UnifyTypes<Value, Var> for Unifier<'_> {
    fn unify_types(
        &mut self,
        lhs: &Value,
        Var(tvar, bounds): &Var,
    ) -> Result<Arc<Type>, TypeError> {
        self.unify_with_type_var(Type::Value(lhs.clone()).into(), *tvar, bounds)
    }
}

impl UnifyTypes<AssociatedType, Var> for Unifier<'_> {
    fn unify_types(
        &mut self,
        associated: &AssociatedType,
        var: &Var,
    ) -> Result<Arc<Type>, TypeError> {
        if let Some(resolved_ty) = associated.resolve_selector_target(self)? {
            self.unify(resolved_ty, var.clone().into())
        } else {
            Ok(AssociatedType {
                impl_ty: associated.impl_ty.clone(),
                selector: associated.selector.clone(),
                resolved_ty: self.unify(associated.resolved_ty.clone(), var.clone().into())?,
            }
            .into())
        }
    }
}

impl UnifyTypes<AssociatedType, AssociatedType> for Unifier<'_> {
    fn unify_types(
        &mut self,
        lhs: &AssociatedType,
        rhs: &AssociatedType,
    ) -> Result<Arc<Type>, TypeError> {
        Ok(AssociatedType {
            impl_ty: self.unify(lhs.impl_ty.clone(), rhs.impl_ty.clone())?,
            selector: if lhs.selector == rhs.selector {
                lhs.selector.clone()
            } else {
                Err(TypeError::Conflict(format!(
                    "Cannot unify associated types {} and {}",
                    lhs, rhs
                )))?
            },
            resolved_ty: self.unify(lhs.resolved_ty.clone(), rhs.resolved_ty.clone())?,
        }
        .into())
    }
}

impl UnifyTypes<AssociatedType, Value> for Unifier<'_> {
    fn unify_types(
        &mut self,
        assoc: &AssociatedType,
        value: &Value,
    ) -> Result<Arc<Type>, TypeError> {
        // If the associated type is resolved then unify the resolved value with the value arg, else unify to a
        // new associated type where the unresolved type is unified with the value.

        if let Some(resolved_value) = assoc.resolve_selector_target(self)? {
            self.unify(value.clone().into(), resolved_value)
        } else {
            Ok(AssociatedType {
                impl_ty: assoc.impl_ty.clone(),
                selector: assoc.selector.clone(),
                resolved_ty: self.unify(assoc.resolved_ty.clone(), value.clone().into())?,
            }
            .into())
        }
    }
}

impl UnifyTypes<Var, Var> for Unifier<'_> {
    fn unify_types(&mut self, lhs: &Var, rhs: &Var) -> Result<Arc<Type>, TypeError> {
        let Var(lhs_tvar, lhs_bounds) = lhs;
        let Var(rhs_tvar, rhs_bounds) = rhs;

        match (self.get_type(*lhs_tvar), self.get_type(*rhs_tvar)) {
            (None, None) => {
                let merged_bounds = lhs_bounds.union(rhs_bounds);
                let unified = self.fresh_bounded_tvar(merged_bounds);
                self.substitute(*lhs_tvar, unified.clone());
                self.substitute(*rhs_tvar, unified.clone());
                Ok(unified)
            }

            (None, Some(rhs)) => {
                self.satisfy_bounds(&rhs, lhs_bounds)?;
                self.substitute(*lhs_tvar, rhs.clone());
                Ok(rhs)
            }

            (Some(lhs), None) => {
                self.satisfy_bounds(&lhs, rhs_bounds)?;
                self.substitute(*rhs_tvar, lhs.clone());
                Ok(lhs)
            }

            (Some(lhs), Some(rhs)) => self.unify(lhs, rhs),
        }
    }
}

impl UnifyTypes<Projection, Projection> for Unifier<'_> {
    fn unify_types(&mut self, lhs: &Projection, rhs: &Projection) -> Result<Arc<Type>, TypeError> {
        if lhs.len() == rhs.len() {
            let mut cols: Vec<ProjectionColumn> = Vec::with_capacity(lhs.len());

            for (lhs_col, rhs_col) in lhs
                .columns()
                .iter()
                .zip(rhs.columns())
            {
                let unified_ty = self.unify(lhs_col.ty.clone(), rhs_col.ty.clone())?;
                cols.push(ProjectionColumn::new(unified_ty, lhs_col.alias.clone()));
            }

            Ok(Projection::new(cols).into())
        } else {
            Err(TypeError::Conflict(format!(
                "cannot unify projections {} and {} because they have different numbers of columns",
                lhs, rhs
            )))
        }
    }
}

impl UnifyTypes<NativeValue, NativeValue> for Unifier<'_> {
    fn unify_types(
        &mut self,
        lhs: &NativeValue,
        rhs: &NativeValue,
    ) -> Result<Arc<Type>, TypeError> {
        match (lhs, rhs) {
            (NativeValue(Some(_)), NativeValue(Some(_)))
            | (NativeValue(Some(_)), NativeValue(None)) => Ok(Type::from(lhs.clone()).into()),

            (NativeValue(None), NativeValue(Some(_))) => Ok(Type::from(rhs.clone()).into()),

            _ => Ok(Type::from(lhs.clone()).into()),
        }
    }
}
