/// A type variable generator.
///
/// Every time the [`Unifier`] sees a [`TypeVar::Fresh`] it replaces it with a `TypeVar::Assigned(_)` using the next
/// `u32` in the sequence.
#[derive(Debug, Default)]
pub(crate) struct TypeVarGenerator(u32);

impl TypeVarGenerator {
    /// Creates a new `TypeVarGenerator`.
    pub(crate) fn new() -> Self {
        Self(0)
    }

    /// Gets the next type variable.
    pub(crate) fn next_tvar(&mut self) -> u32 {
        let next_id = self.0;
        self.0 += 1;
        next_id
    }
}
