use std::{cell::RefCell, rc::Rc, sync::Arc};

/// A shared dependency of type `T`, convertible to an `Rc<RefCell<T>>`.
///
/// This exists purely to avoid boilerplate.
pub struct DepMut<T> {
    shared: Rc<RefCell<T>>,
}

impl<T> DepMut<T> {
    pub fn new(dep: T) -> Self {
        Self {
            shared: Rc::new(RefCell::new(dep)),
        }
    }
}

impl<T> From<&DepMut<T>> for Rc<RefCell<T>> {
    fn from(dep: &DepMut<T>) -> Self {
        dep.shared.clone()
    }
}

impl<T> From<DepMut<T>> for Rc<RefCell<T>> {
    fn from(dep: DepMut<T>) -> Self {
        dep.shared.clone()
    }
}

/// A shared dependency of type `T`, convertible to an `Arc<T>`.
///
/// This exists purely to avoid boilerplate.
pub struct Dep<T> {
    shared: Arc<T>,
}

impl<T> Dep<T> {
    pub fn new(dep: T) -> Self {
        Self {
            shared: Arc::new(dep),
        }
    }
}

impl<T> From<Arc<T>> for Dep<T> {
    fn from(value: Arc<T>) -> Self {
        Dep { shared: value }
    }
}

impl<T> From<&Dep<T>> for Arc<T> {
    fn from(dep: &Dep<T>) -> Self {
        dep.shared.clone()
    }
}
