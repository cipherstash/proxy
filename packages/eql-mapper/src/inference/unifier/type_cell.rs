use std::{cell::RefCell, rc::Rc};

use super::{Constructor, Type, TypeError, TypeRegistry, Value};

/// `TypeCell` is a shareable mutable container of a [`Type`].
///
/// It used by [`super::Unifier`] to ensure that two successfully unified types share the same allocation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeCell {
    shared_ty: Rc<RefCell<SharedTypeMut>>,
}

impl TypeCell {
    /// Create a new `TypeCell` that owns a [`Type`].
    pub(crate) fn new(ty: Type) -> Self {
        Self {
            shared_ty: Rc::new(RefCell::new(SharedTypeMut::new(ty))),
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
            *self.shared_ty.borrow_mut() = other.shared_ty.borrow().clone();
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
        self.shared_ty.borrow().set_type(ty);
    }

    /// Returns the [`Rc<Type>`] that underlying `self`.
    pub fn as_type(&self) -> Rc<Type> {
        self.shared_ty.borrow().as_type()
    }

    /// Tests whether `self` and `other` share a mutable type allocation.
    pub fn has_same_pointee(&self, other: &Self) -> bool {
        self.shared_ty
            .borrow()
            .has_same_pointee(&other.shared_ty.borrow())
    }

    /// Resolves the `Type` stored in `self`, returning it as an [`Rc<Type>`].
    ///
    /// A resolved type is one in which all type variables have been resolved, recursively.
    ///
    /// Fails with a [`TypeError`] if the stored `Type` cannot be fully resolved.
    pub fn resolved(&self, registry: &TypeRegistry<'_>) -> Result<Rc<Type>, TypeError> {
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
                None => Err(TypeError::Incomplete(format!(
                    "type {} contains unresolved type variables",
                    *ty
                ))),
            },
        }
    }
}

type SharedType = Rc<Type>;

#[derive(Debug, Clone, PartialEq, Eq)]
struct SharedTypeMut {
    alloc: Rc<RefCell<SharedType>>,
}

impl SharedTypeMut {
    fn new(ty: Type) -> Self {
        Self {
            alloc: Rc::new(RefCell::new(SharedType::new(ty))),
        }
    }

    #[cfg(test)]
    fn set_type(&self, ty: Type) {
        *self.alloc.borrow_mut() = SharedType::new(ty);
    }

    fn as_type(&self) -> Rc<Type> {
        self.alloc.borrow().clone()
    }

    fn has_same_pointee(&self, other: &Self) -> bool {
        Rc::ptr_eq(&self.alloc.borrow(), &other.alloc.borrow())
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
