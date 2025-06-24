use crate::TypeError;

use super::{Array, NativeValue, Projection, SetOf, Type, Unifier, Value, Var};

/// A trait for resolving all type variables contained in a [`crate::unifier::Type`] and converting the successfully
/// resolved type into the publicly exported [`crate::Type`] type representation which is identical except for the
/// absence of type variables.
pub(crate) trait ResolveType {
    /// The corresponding type for `Self` in `crate::Type::..`, e.g. when `Self` is `crate::unifier::Type` then
    /// `Self::Output` is `crate::Type`.
    type Output;

    /// Recursively resolves all type variables found in `self` and if successful it builds and returns `Ok(Self::Output)`.
    ///
    /// Returns a [`TypeError`] if there are any unresolved type variables.
    fn resolve_type(&self, unifier: &mut Unifier<'_>) -> Result<Self::Output, TypeError>;
}

impl ResolveType for Type {
    type Output = crate::Type;

    fn resolve_type(&self, unifier: &mut Unifier<'_>) -> Result<Self::Output, TypeError> {
        match self {
            Type::Value(constructor) => Ok(constructor.resolve_type(unifier)?.into()),

            Type::Var(Var(type_var, _)) => {
                if let Some(sub_ty) = unifier.get_type(*type_var) {
                    return sub_ty.resolved(unifier);
                }

                Err(TypeError::Incomplete(format!(
                    "there are no substitutions for type var {}",
                    type_var
                )))
            }

            Type::Associated(associated) => {
                if let Some(constructor) = associated.resolve_selector_target(unifier)? {
                    Ok(constructor.resolve_type(unifier)?)
                } else {
                    Err(TypeError::InternalError(format!(
                        "could not resolve associated type {}",
                        associated
                    )))
                }
            }
        }
    }
}

impl ResolveType for SetOf {
    type Output = crate::SetOf;

    fn resolve_type(&self, unifier: &mut Unifier<'_>) -> Result<Self::Output, TypeError> {
        Ok(crate::SetOf(Box::new(self.0.resolve_type(unifier)?)))
    }
}

impl ResolveType for Value {
    type Output = crate::Value;

    fn resolve_type(&self, unifier: &mut Unifier<'_>) -> Result<Self::Output, TypeError> {
        match self {
            Value::Eql(eql_term) => Ok(crate::Value::Eql(eql_term.clone())),
            Value::Native(NativeValue(Some(native_col))) => {
                Ok(crate::Value::Native(NativeValue(Some(native_col.clone()))))
            }
            Value::Native(NativeValue(None)) => Ok(crate::Value::Native(NativeValue(None))),
            Value::Array(Array(element_ty)) => {
                let resolved = element_ty.resolve_type(unifier)?;
                Ok(crate::Value::Array(crate::Array(resolved.into())))
            }
            Value::Projection(projection) => {
                Ok(crate::Value::Projection(projection.resolve_type(unifier)?))
            }

            Value::SetOf(set_of) => {
                let resolved = set_of.resolve_type(unifier)?;
                Ok(crate::Value::SetOf(resolved))
            }
        }
    }
}

impl ResolveType for Projection {
    type Output = crate::Projection;

    fn resolve_type(&self, unifier: &mut Unifier<'_>) -> Result<Self::Output, TypeError> {
        let resolved_cols = self
            .flatten()
            .columns()
            .iter()
            .map(|col| -> Result<crate::ProjectionColumn, TypeError> {
                let alias = col.alias.clone();
                let ty = col.ty.resolve_type(unifier)?;

                if let crate::Type::Value(crate::Value::Projection(projection)) = ty {
                    return Err(TypeError::Expected(format!(
                        "projection not flattened: {}",
                        projection
                    )));
                }

                let crate::Type::Value(value) = ty;
                Ok(crate::ProjectionColumn { ty: value, alias })
            })
            .collect::<Result<Vec<_>, _>>()?;

        if resolved_cols.is_empty() {
            Ok(crate::Projection::Empty)
        } else {
            Ok(crate::Projection::WithColumns(resolved_cols))
        }
    }
}
