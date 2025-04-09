use core::ops::Deref;
use std::{fmt::Debug, hash::Hash, rc::Rc};

/// An `RcRef<U>` is a reference to a value owned by an [`Rc<T>`] and is created by calling [`RcMap::map`] on the
/// [`RcMap`] trait. `RcMap` is implemented only for `Rc`. See [`crate::ArcRef`] for the `Arc` equivalent.
///
/// More formally, an `RcRef<U>` is a reference to a `U` owned by the `T` inside an `Rc<T>`.
///
/// `RcRef<U>` implements [`Deref<Target = U>`] so it can be used anywhere a `&U` can be used.
///
/// `RcRef<U>` contributes to the reference count of the original `Rc<T>` which prevents the `T` (and by implication, the
/// `U`) from being dropped.
///
/// # Example
///
/// ```
/// use std::rc::Rc;
/// use crate::RcMap;
///
/// let msg: Rc<String> = Rc::new(String::from("Hello!"));
/// let rc_ref: RcRef<&str> = msg.map(|s| s.as_str());
/// ```
/// See: [`std::cell::Ref::map`] for a similar idea but for [`std::cell::RefCell`].
#[derive(Clone)]
pub struct RcRef<U: ?Sized> {
    value_addr: *const U,
    on_drop: Rc<dyn Fn()>,
}

mod sealed {
    pub(crate) trait Private {}
}

/// Extension trait for [`Rc`].  Allows [`RcMap::map`] to be invoked directly on an `Rc`.
#[allow(private_bounds)]
pub trait RcMap<T: ?Sized + 'static, U: ?Sized>: sealed::Private {
    /// Create an [`RcRef<U>`] from an [`Rc<T>`] or `&Rc<T>` where `&U` is a reference to a value directly or
    /// transitively owned by `T`.
    ///
    /// Invoke `map(&some_rc)` to retain ownership of `some_rc`.
    ///
    /// Invoke `map(some_rc)` to relinquish ownership of `some_rc`.
    fn map(self, projection: fn(&T) -> &U) -> RcRef<U>;

    fn try_map(self, projection: fn(&T) -> Result<&U, ()>) -> Result<RcRef<U>, ()>;
}

impl<T: ?Sized + 'static> sealed::Private for Rc<T> {}
impl<T: ?Sized + 'static> sealed::Private for &Rc<T> {}

impl<T: ?Sized + 'static, U: ?Sized> RcMap<T, U> for Rc<T> {
    fn map(self, projection: fn(&T) -> &U) -> RcRef<U> {
        // Get a &U but cast to *const U to erase the lifetime.
        let value_addr: *const U = projection(&*self) as *const U;

        // Get raw pointer to content of Rc<T> and capture it in the closure which means RcRef<U> does not need to be
        // RcRef<T, U> (specifically, it erases the type T).  Rc::into_raw preserves the strong count which means it is
        // always safe to convert rc_addr back to Rc<T> in the closure.
        let rc_addr = Rc::into_raw(self) as *const T;

        RcRef {
            value_addr,
            on_drop: Rc::new(move || unsafe { Rc::decrement_strong_count(rc_addr) }),
        }
    }

    fn try_map(self, projection: fn(&T) -> Result<&U, ()>) -> Result<RcRef<U>, ()> {
        let value_addr: *const U = projection(&*self)? as *const U;
        let rc_addr = Rc::into_raw(self) as *const T;

        Ok(RcRef {
            value_addr,
            on_drop: Rc::new(move || unsafe { Rc::decrement_strong_count(rc_addr) }),
        })
    }
}

impl<T: ?Sized + 'static> From<Rc<T>> for RcRef<T> {
    fn from(value: Rc<T>) -> Self {
        value.map(|itself| itself)
    }
}

impl<T: ?Sized + 'static> From<&Rc<T>> for RcRef<T> {
    fn from(value: &Rc<T>) -> Self {
        value.clone().map(|itself| itself)
    }
}

impl<T: ?Sized + 'static, U: ?Sized> RcMap<T, U> for &Rc<T> {
    fn map(self, projection: fn(&T) -> &U) -> RcRef<U> {
        self.clone().map(projection)
    }

    fn try_map(self, projection: fn(&T) -> Result<&U, ()>) -> Result<RcRef<U>, ()> {
        self.clone().try_map(projection)
    }
}

impl<U: ?Sized> Deref for RcRef<U> {
    type Target = U;

    fn deref(&self) -> &Self::Target {
        // SAFETY: &U is a reference to data to within the original Rc<T> which we have retained a strong reference to,
        // so it cannot have been dropped. The &U cannot have been mutated or moved so it is safe to cast the *const U
        // back to a &U.
        unsafe { &*self.value_addr as &U }
    }
}

impl<U: ?Sized> Drop for RcRef<U> {
    fn drop(&mut self) {
        (*self.on_drop)()
    }
}

impl<U: ?Sized> Eq for RcRef<U> where U: PartialEq {}

impl<U: ?Sized> PartialEq for RcRef<U>
where
    U: PartialEq,
{
    fn eq(&self, other: &Self) -> bool {
        **self == **other
    }
}

impl<U: ?Sized> Ord for RcRef<U>
where
    U: Ord,
{
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (**self).cmp(&**other)
    }
}

impl<U: ?Sized> PartialOrd for RcRef<U>
where
    U: PartialOrd,
{
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        (**self).partial_cmp(&**other)
    }
}

impl<U: ?Sized> Hash for RcRef<U>
where
    U: Hash,
{
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        (**self).hash(state);
    }
}

impl<U: ?Sized> Debug for RcRef<U>
where
    U: Debug,
{
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (**self).fmt(f)
    }
}

#[cfg(test)]
mod test {
    use super::RcMap;
    use std::rc::Rc;

    #[test]
    fn rcref_via_owned_rc() {
        let rc = Rc::new(String::from("Hello!"));
        let rcref = rc.map(|s| s.as_str());

        assert_eq!(&*rcref, "Hello!");
    }

    #[test]
    fn rc_ref_via_borrowed_rc() {
        let rc = &Rc::new(String::from("Hello!"));
        let rcref = rc.map(|s| s.as_str());

        assert_eq!(&*rcref, "Hello!");
        assert_eq!(rc.as_str(), "Hello!");

        assert!(std::ptr::eq(
            &*rcref as *const str,
            rc.as_str() as *const str
        ));
    }

    #[test]
    fn rc_strong_count() {
        let rc = &Rc::new(String::from("Hello!"));
        assert_eq!(Rc::strong_count(rc), 1);

        let rcref = rc.map(|s| s.as_str());
        assert_eq!(Rc::strong_count(rc), 2);

        drop(rcref);
        assert_eq!(Rc::strong_count(rc), 1);
    }
}
