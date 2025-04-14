use core::ops::Deref;
use std::{fmt::Debug, hash::Hash, sync::Arc};

/// An `ArcRef<U>` is a reference to a value owned by an [`Arc<T>`] and is created by calling [`ArcMap::map`] on the
/// [`ArcMap`] trait. `ArcMap` is implemented only for `Arc`.
///
/// `ArcRef<U>` implements [`Deref<Target = U>`] so it can be used anywhere a `&U` can be used.
///
/// `ArcRef<U>` contributes to the reference count of the original `Arc<T>` which prevents the `T` (and by implication, the
/// `U`) from being dropped.
///
/// # Example
///
/// ```
/// use std::arc::Arc;
/// use crate::ArcMap;
///
/// let msg: Arc<String> = Arc::new(String::from("Hello!"));
/// let arc_ref: ArcRef<&str> = msg.map(|s| s.as_str());
/// ```
/// See: [`std::cell::Ref::map`] for a similar idea but for [`std::cell::RefCell`].
pub struct ArcRef<U: ?Sized> {
    value_addr: *const U,
    update_strong_count: UpdateStrongCountFn,
}

type UpdateStrongCountFn = Arc<dyn Fn(bool) + Send + Sync + 'static>;

mod sealed {
    pub(crate) trait Private {}
}

/// Extension trait for [`Arc`].  Allows [`ArcMap::map`] to be invoked directly on an `Arc`.
#[allow(private_bounds)]
pub trait ArcMap<T: ?Sized + 'static, U: ?Sized>: sealed::Private {
    /// Create an [`ArcRef<U>`] from an [`Arc<T>`] or `&Arc<T>` where `&U` is a reference to a value directly or
    /// transitively owned by `T`.
    ///
    /// Invoke `map(&some_rc)` to retain ownership of `some_rc`.
    ///
    /// Invoke `map(some_rc)` to relinquish ownership of `some_rc`.
    fn map(self, projection: fn(&T) -> &U) -> ArcRef<U>;

    fn try_map(self, projection: fn(&T) -> Result<&U, ()>) -> Result<ArcRef<U>, ()>;
}

impl<T: ?Sized + 'static> sealed::Private for Arc<T> {}
impl<T: ?Sized + 'static> sealed::Private for &Arc<T> {}

impl<T: ?Sized + 'static, U: ?Sized> ArcMap<T, U> for Arc<T>
where
    T: Send + Sync,
{
    fn map(self, projection: fn(&T) -> &U) -> ArcRef<U> {
        // Get a &U but cast to *const U to erase the lifetime.
        let value_addr: *const U = projection(&*self) as *const U;

        // Get raw pointer to content of Arc<T> and capture it in the closure which means ArcRef<U> does not need to be
        // ArcRef<T, U> (specifically, it erases the type T).  Arc::into_raw preserves the strong count which means it is
        // always safe to convert arc_addr back to Arc<T> in the closure.
        let arc_addr = Arc::into_raw(self);
        let arc_addr = unsafe { &*arc_addr as &T };

        ArcRef {
            value_addr,
            update_strong_count: Arc::new(move |inc_or_dec: bool| {
                if inc_or_dec {
                    unsafe {
                        Arc::increment_strong_count(arc_addr);
                    }
                } else {
                    unsafe {
                        Arc::decrement_strong_count(arc_addr);
                    }
                }
            }),
        }
    }

    fn try_map(self, projection: fn(&T) -> Result<&U, ()>) -> Result<ArcRef<U>, ()> {
        let value_addr: *const U = projection(&*self)? as *const U;
        let arc_addr = Arc::into_raw(self);
        let arc_addr = unsafe { &*arc_addr as &T };

        Ok(ArcRef {
            value_addr,
            update_strong_count: Arc::new(move |inc_or_dec: bool| {
                if inc_or_dec {
                    unsafe {
                        Arc::increment_strong_count(arc_addr);
                    }
                } else {
                    unsafe {
                        Arc::decrement_strong_count(arc_addr);
                    }
                }
            }),
        })
    }
}

impl<T: ?Sized + 'static> From<Arc<T>> for ArcRef<T>
where
    T: Send + Sync,
{
    fn from(value: Arc<T>) -> Self {
        value.map(|itself| itself)
    }
}

impl<T: ?Sized + 'static> From<&Arc<T>> for ArcRef<T>
where
    T: Send + Sync,
{
    fn from(value: &Arc<T>) -> Self {
        value.clone().map(|itself| itself)
    }
}

impl<T: ?Sized + 'static, U: ?Sized> ArcMap<T, U> for &Arc<T>
where
    T: Send + Sync,
{
    fn map(self, projection: fn(&T) -> &U) -> ArcRef<U> {
        Arc::clone(self).map(projection)
    }

    fn try_map(self, projection: fn(&T) -> Result<&U, ()>) -> Result<ArcRef<U>, ()> {
        Arc::clone(self).try_map(projection)
    }
}

unsafe impl<U: ?Sized> Send for ArcRef<U> {}
unsafe impl<U: ?Sized> Sync for ArcRef<U> {}

impl<U: ?Sized> Clone for ArcRef<U> {
    fn clone(&self) -> Self {
        (*self.update_strong_count)(true);
        Self {
            value_addr: self.value_addr,
            update_strong_count: self.update_strong_count.clone(),
        }
    }
}

impl<U: ?Sized> Deref for ArcRef<U> {
    type Target = U;

    fn deref(&self) -> &Self::Target {
        // SAFETY: &U is a reference to data to within the original Arc<T> which we have retained a strong reference to,
        // so it cannot have been dropped. The &U cannot have been mutated or moved so it is safe to cast the *const U
        // back to a &U.
        unsafe { &*self.value_addr as &U }
    }
}

impl<U: ?Sized> Drop for ArcRef<U> {
    fn drop(&mut self) {
        (*self.update_strong_count)(false)
    }
}

impl<U: ?Sized> Eq for ArcRef<U> where U: PartialEq {}

impl<U: ?Sized> PartialEq for ArcRef<U>
where
    U: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<U: ?Sized> Ord for ArcRef<U>
where
    U: Ord,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (**self).cmp(&**other)
    }
}

impl<U: ?Sized> PartialOrd for ArcRef<U>
where
    U: PartialOrd,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        (**self).partial_cmp(&**other)
    }
}

impl<U: ?Sized> Hash for ArcRef<U>
where
    U: Hash,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

impl<U: ?Sized> Debug for ArcRef<U>
where
    U: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (**self).fmt(f)
    }
}

#[cfg(test)]
mod test {
    use super::ArcMap;
    use std::sync::Arc;

    #[test]
    fn arc_ref_via_owned_rc() {
        let arc = Arc::new(String::from("Hello!"));
        let arc_ref = arc.map(|s| s.as_str());

        assert_eq!(&*arc_ref, "Hello!");
    }

    #[test]
    fn arc_ref_via_borrowed_rc() {
        let arc = &Arc::new(String::from("Hello!"));
        let arc_ref = arc.map(|s| s.as_str());

        assert_eq!(&*arc_ref, "Hello!");
        assert_eq!(arc.as_str(), "Hello!");

        assert!(std::ptr::eq(
            &*arc_ref as *const str,
            arc.as_str() as *const str
        ));
    }

    #[test]
    fn arc_strong_count() {
        let arc = &Arc::new(String::from("Hello!"));
        assert_eq!(Arc::strong_count(arc), 1);

        let arc_ref = arc.map(|s| s.as_str());
        assert_eq!(Arc::strong_count(arc), 2);

        drop(arc_ref);
        assert_eq!(Arc::strong_count(arc), 1);

        let arc_ref = arc.map(|s| s.as_str());
        assert_eq!(Arc::strong_count(arc), 2);
        let cloned = arc_ref.clone();
        assert_eq!(&*cloned, "Hello!");

        assert_eq!(Arc::strong_count(arc), 3);
    }
}
