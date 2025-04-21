use std::marker::PhantomData;

use super::{unifier::TypeVar};

#[derive(Debug)]
pub(crate) struct Sequence<T> {
    next_value: usize,
    _marker: PhantomData<T>,
}

impl<T> Sequence<T> {
    pub(crate) fn new() -> Self {
        Self { next_value: 0, _marker: PhantomData }
    }
}

impl Sequence<TypeVar> {
    pub(crate) fn next_value(&mut self) -> TypeVar {
        let t = TypeVar(self.next_value);
        self.next_value += 1;
        t
    }
}
