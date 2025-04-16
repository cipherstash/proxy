use std::marker::PhantomData;

use super::{unifier::TypeVar, TID};

#[derive(Debug)]
pub(crate) struct Sequence<T> {
    next_value: u32,
    _marker: PhantomData<T>,
}

pub(crate) struct SequenceVal<T> {
    pub(crate) value: u32,
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

impl Sequence<TID> {
    pub(crate) fn new_starting_at(id: u32) -> Self {
        Self { next_value: id, _marker: PhantomData }
    }

    pub(crate) fn next_value(&mut self) -> SequenceVal<TID> {
        let t = SequenceVal { value: self.next_value, _marker: PhantomData };
        self.next_value += 1;
        t
    }
}
