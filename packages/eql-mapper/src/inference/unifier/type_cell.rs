use std::{any::type_name, sync::Arc, sync::RwLock};

use crate::{ArcMap, ArcRef};

use super::{Constructor, NativeValue, Type, TypeError, TypeRegistry, Value};

/// `TypeCell` is a shareable mutable container of a [`Type`].
///
/// It used by [`super::Unifier`] to ensure that two successfully unified types share the same allocation.
#[derive(Debug, Clone)]
pub struct TypeCell {
    shared_ty: Arc<RwLock<SharedTypeMut>>,
}

impl Eq for TypeCell {}

impl PartialEq for TypeCell {
    fn eq(&self, other: &Self) -> bool {
        *self.shared_ty.read().unwrap() == *other.shared_ty.read().unwrap()
    }
}

impl TypeCell {
    /// Create a new `TypeCell` that owns a [`Type`].
    pub(crate) fn new(ty: Type) -> Self {
        Self {
            shared_ty: Arc::new(RwLock::new(SharedTypeMut::new(ty))),
        }
    }

    /// Replaces the `Type` allocation in `self` with a shared clone of the allocation from `other`.  The original
    /// `Type` allocation in `self` will no long be reachable and will be dropped automatically.
    ///
    /// After the join, an update to the `Type` allocation of `other` will also be seen by `self` and vice versa.
    ///
    /// Joining is transitive, so `a.join(&b.join(&c))` will make `a`, `b` and `c` all share a mutable type allocation.
    pub(crate) fn join(&self, other: &TypeCell) -> TypeCell {
        // If allocations are already shared then skip this part (which also prevents BorrowMutError).
        if !self.has_same_pointee(other) {
            *self.shared_ty.write().unwrap() = other.shared_ty.read().unwrap().clone();
        }

        self.clone()
    }

    /// Replaces the `Type` allocation in `self` with
    pub(crate) fn join_all(&self, others: &[&TypeCell]) -> TypeCell {
        if let [first, rest @ ..] = others {
            self.join(&rest.iter().fold((*first).clone(), |acc, tc| acc.join(tc)));
        }
        self.clone()
    }

    /// Sets the [`Type`] that `self` will resolve to.
    ///
    /// All other `TypeCell`s that `self` has been bound with (see: [`Self::join`]) will also resolve to the same `Type`.
    #[cfg(test)]
    pub(crate) fn set_type(&self, ty: Type) {
        self.shared_ty.write().unwrap().set_type(ty);
    }

    /// Returns the [`Arc<Type>`] that underlying `self`.
    pub fn as_type(&self) -> Arc<Type> {
        self.shared_ty.read().unwrap().as_type()
    }

    pub fn is_eql_value(&self) -> bool {
        matches!(
            *self.as_type(),
            Type::Constructor(Constructor::Value(Value::Eql(_)))
        )
    }

    /// Tests whether `self` and `other` share a mutable type allocation.
    pub fn has_same_pointee(&self, other: &Self) -> bool {
        self.shared_ty
            .read()
            .unwrap()
            .has_same_pointee(&other.shared_ty.read().unwrap())
    }

    /// Resolves the `Type` stored in `self`, returning it as an [`Arc<Type>`].
    ///
    /// A resolved type is one in which all type variables have been resolved, recursively.
    ///
    /// Fails with a [`TypeError`] if the stored `Type` cannot be fully resolved.
    pub fn resolved(&self, registry: &TypeRegistry<'_>) -> Result<Arc<Type>, TypeError> {
        let ty = self.as_type();
        match &*ty {
            Type::Constructor(constructor) => match constructor {
                Constructor::Value(value) => match value {
                    Value::Eql(_) | Value::Native(_) => Ok(ty),
                    Value::Array(type_cell) => type_cell.resolved(registry).map(|_| ty),
                },
                Constructor::Projection(projection) => {
                    for col in projection.columns() {
                        col.ty.resolved(registry)?;
                    }
                    Ok(ty)
                }
            },
            Type::Var(type_var) => match registry.get_substitution(*type_var) {
                Some(ty_cell) => ty_cell.resolved(registry),
                None => {
                    if !registry.value_expr_exists_with_type(self.clone()) {
                        Err(TypeError::Incomplete(format!(
                            "type {} contains unresolved type variables",
                            *ty
                        )))
                    } else {
                        let updated = self.join(&TypeCell::new(Type::Constructor(
                            Constructor::Value(Value::Native(NativeValue(None))),
                        )));
                        Ok(updated.as_type())
                    }
                }
            },
        }
    }

    pub fn resolved_as<T: Send + Sync + 'static>(
        &self,
        registry: &TypeRegistry<'_>,
    ) -> Result<ArcRef<T>, TypeError> {
        let resolved = &self.resolved(registry)?;
        resolved
            .try_map(|ty| match ty {
                Type::Constructor(Constructor::Projection(projection)) => {
                    if let Some(t) = (projection as &dyn std::any::Any).downcast_ref::<T>() {
                        return Ok(t);
                    }

                    Err(())
                }
                Type::Constructor(Constructor::Value(value)) => {
                    if let Some(t) = (value as &dyn std::any::Any).downcast_ref::<T>() {
                        return Ok(t);
                    }

                    Err(())
                }
                Type::Var(_) => return Err(()),
            })
            .map_err(|_| {
                TypeError::InternalError(format!(
                    "could not resolve type {} as {}",
                    &*resolved,
                    type_name::<T>()
                ))
            })
    }
}

type SharedType = Arc<Type>;

#[derive(Debug, Clone)]
struct SharedTypeMut {
    alloc: Arc<RwLock<SharedType>>,
}

impl Eq for SharedTypeMut {}

impl PartialEq for SharedTypeMut {
    fn eq(&self, other: &Self) -> bool {
        *self.alloc.read().unwrap() == *other.alloc.read().unwrap()
    }
}

impl SharedTypeMut {
    fn new(ty: Type) -> Self {
        Self {
            alloc: Arc::new(RwLock::new(SharedType::new(ty))),
        }
    }

    #[cfg(test)]
    fn set_type(&self, ty: Type) {
        *self.alloc.write().unwrap() = SharedType::new(ty);
    }

    fn as_type(&self) -> Arc<Type> {
        self.alloc.read().unwrap().clone()
    }

    fn has_same_pointee(&self, other: &Self) -> bool {
        Arc::ptr_eq(&*self.alloc.read().unwrap(), &*other.alloc.read().unwrap())
    }
}

#[cfg(test)]
mod tests {
    use crate::unifier::TypeVar;

    use super::{Type, TypeCell};

    #[test]
    fn join_is_transitive() {
        let a = TypeCell::new(Type::Var(TypeVar(0)));
        let b = TypeCell::new(Type::Var(TypeVar(0)));
        let c = TypeCell::new(Type::Var(TypeVar(0)));

        assert!(!a.has_same_pointee(&b));
        assert!(!a.has_same_pointee(&c));
        assert!(!b.has_same_pointee(&c));

        a.join(&b.join(&c));

        assert!(a.has_same_pointee(&b));
        assert!(a.has_same_pointee(&c));
        assert!(b.has_same_pointee(&c));

        a.set_type(Type::Var(TypeVar(1)));

        assert_eq!(&*a.as_type(), &Type::Var(TypeVar(1)));
        assert_eq!(&*b.as_type(), &Type::Var(TypeVar(1)));
        assert_eq!(&*c.as_type(), &Type::Var(TypeVar(1)));
    }

    #[test]
    fn join_all_is_transitive() {
        let a = TypeCell::new(Type::Var(TypeVar(0)));
        let b = TypeCell::new(Type::Var(TypeVar(0)));
        let c = TypeCell::new(Type::Var(TypeVar(0)));

        assert!(!a.has_same_pointee(&b));
        assert!(!a.has_same_pointee(&c));
        assert!(!b.has_same_pointee(&c));

        a.join_all(&[&b, &c]);

        assert!(a.has_same_pointee(&b));
        assert!(a.has_same_pointee(&c));
        assert!(b.has_same_pointee(&c));

        a.set_type(Type::Var(TypeVar(1)));

        assert_eq!(&*a.as_type(), &Type::Var(TypeVar(1)));
        assert_eq!(&*b.as_type(), &Type::Var(TypeVar(1)));
        assert_eq!(&*c.as_type(), &Type::Var(TypeVar(1)));
    }
}
