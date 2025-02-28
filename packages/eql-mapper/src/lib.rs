//! `eql-mapper` transforms SQL to SQL+EQL using a known database schema as a reference.

mod eql_mapper;
mod importer;
mod inference;
mod iterator_ext;
mod model;
mod scope;

use std::{cell::RefCell, rc::Rc, sync::Arc};

pub use eql_mapper::*;
pub use importer::*;
pub use inference::*;
pub use model::*;
pub use scope::*;

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
