//! The [`UnifyTypes`] trait definition and all of the implementations.
//!
//! The entry point for [`Type`] unification is [`Unifier::unify`] which is an inherent method on the [`Unifier`] itself
//! and not part of the `UnifyTypes` trait.

use std::sync::Arc;

use crate::TypeError;

use super::{
    Constructor, EqlTerm, NativeValue, Projection, ProjectionColumn, Type, Unifier, Value, Var,
};

/// Trait for unifying two types.
///
/// The `Lhs` and `Rhs` type arguments are independenty specifiable because some different base types (such as `Var` +
/// `Constructor` and `Value` + `Projection`) can be unified.
pub(super) trait UnifyTypes<Lhs, Rhs> {
    /// Try to unify types `lhs` & `rhs` to produce a new [`Type`].
    fn unify_types(&mut self, lhs: &Lhs, rhs: &Rhs) -> Result<Arc<Type>, TypeError>;
}

impl UnifyTypes<Constructor, Constructor> for Unifier<'_> {
    fn unify_types(
        &mut self,
        lhs: &Constructor,
        rhs: &Constructor,
    ) -> Result<Arc<Type>, TypeError> {
        match (lhs, rhs) {
            (Constructor::Value(lhs_v), Constructor::Value(rhs_v)) => {
                self.unify_types(lhs_v, rhs_v)
            }

            (Constructor::Value(value), Constructor::Projection(projection))
            | (Constructor::Projection(projection), Constructor::Value(value)) => {
                self.unify_types(value, projection)
            }

            (Constructor::Projection(lhs), Constructor::Projection(rhs)) => {
                self.unify_types(lhs, rhs)
            }
        }
    }
}

impl UnifyTypes<Value, Projection> for Unifier<'_> {
    fn unify_types(&mut self, lhs: &Value, rhs: &Projection) -> Result<Arc<Type>, TypeError> {
        let projection = rhs.flatten();
        let len = projection.len();
        if len == 1 {
            self.unify_types(lhs, &projection[0].ty)
        } else {
            Err(TypeError::Conflict(
                "cannot unify value type with projection of more than one column".to_string(),
            ))
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

            (Value::Array(lhs), Value::Array(rhs)) => self.unify(lhs.clone(), rhs.clone()),

            (lhs, rhs) => Err(TypeError::Conflict(format!(
                "cannot unify values {} and {}",
                lhs, rhs
            ))),
        }
    }
}

impl UnifyTypes<EqlTerm, EqlTerm> for Unifier<'_> {
    fn unify_types(&mut self, lhs: &EqlTerm, rhs: &EqlTerm) -> Result<Arc<Type>, TypeError> {
        match (lhs, rhs) {
            (EqlTerm::Whole(lhs), EqlTerm::Whole(rhs)) if lhs == rhs => {
                Ok(EqlTerm::Whole(lhs.clone()).into())
            }

            (EqlTerm::Whole(whole), EqlTerm::Partial(partial, bounds))
            | (EqlTerm::Partial(partial, bounds), EqlTerm::Whole(whole))
                if whole == partial =>
            {
                let unified = Arc::<Type>::from(EqlTerm::Whole(whole.clone()));
                // self.substitute_all_tvars_pointing_to_target(
                //     EqlTerm::Partial(partial.clone(), bounds.clone()).into(),
                //     unified.clone(),
                // );
                Ok(unified)
            }

            (
                EqlTerm::FixedPartial(lhs_eql_value, lhs_bounds),
                EqlTerm::FixedPartial(rhs_eql_value, rhs_bounds),
            ) if lhs_eql_value == rhs_eql_value && lhs_bounds == rhs_bounds => {
                Ok(EqlTerm::FixedPartial(lhs_eql_value.clone(), lhs_bounds.clone()).into())
            }

            (_, _) => Err(TypeError::Conflict(format!(
                "cannot unify EQL terms {} and {}",
                lhs, rhs
            ))),
        }
    }
}

impl UnifyTypes<Constructor, Var> for Unifier<'_> {
    fn unify_types(
        &mut self,
        lhs: &Constructor,
        Var(tvar, bounds): &Var,
    ) -> Result<Arc<Type>, TypeError> {
        Ok(self.unify_with_type_var(Type::Constructor(lhs.clone()).into(), *tvar, bounds)?)
    }
}

impl UnifyTypes<Var, Var> for Unifier<'_> {
    fn unify_types(&mut self, lhs: &Var, rhs: &Var) -> Result<Arc<Type>, TypeError> {
        let Var(lhs_tvar, lhs_bounds) = lhs;
        let Var(rhs_tvar, rhs_bounds) = rhs;

        let merged_bounds = lhs_bounds.union(&rhs_bounds);

        match (self.get_type(*lhs_tvar), self.get_type(*rhs_tvar)) {
            (None, None) => {
                let unified = self.fresh_bounded_tvar(merged_bounds);
                self.substitute(*lhs_tvar, unified.clone());
                self.substitute(*rhs_tvar, unified.clone());
                Ok(unified)
            },
            (None, Some(rhs)) => {
                self.satisfy_bounds(&*rhs, lhs_bounds)?;
                self.substitute(*lhs_tvar, rhs.clone());
                Ok(rhs)
            },
            (Some(lhs), None) => {
                self.satisfy_bounds(&*lhs, rhs_bounds)?;
                self.substitute(*rhs_tvar, lhs.clone());
                Ok(lhs)
            },
            (Some(lhs), Some(rhs)) => {
               self.unify(lhs, rhs)
            },
        }
    }
}

impl UnifyTypes<Projection, Projection> for Unifier<'_> {
    fn unify_types(&mut self, lhs: &Projection, rhs: &Projection) -> Result<Arc<Type>, TypeError> {
        let lhs_projection = lhs.flatten();
        let rhs_projection = rhs.flatten();

        if lhs_projection.len() == rhs_projection.len() {
            let mut cols: Vec<ProjectionColumn> = Vec::with_capacity(lhs_projection.len());

            for (lhs_col, rhs_col) in lhs_projection
                .columns()
                .iter()
                .zip(rhs_projection.columns())
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
