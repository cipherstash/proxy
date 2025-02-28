/// [`Iterator`] extension methods.
pub(crate) trait IteratorExt: Iterator {
    /// Find an item that matches `predicate` but only if no other item matches.
    ///
    /// - If a unique match is found, returns `Ok(item)`
    /// - If a non-unique match is found, returns `Err(FindUniqueMatchError::NotUnique)`
    /// - If a no match is found, returns `Err(FindUniqueMatchError::NotFound)`
    fn find_unique<P>(&mut self, predicate: &P) -> Result<Self::Item, FindUniqueMatchError>
    where
        Self: Sized,
        P: for<'a> Fn(&'a Self::Item) -> bool,
    {
        match (self.find(predicate), self.find(predicate)) {
            (Some(column), None) => Ok(column),
            (None, _) => Err(FindUniqueMatchError::NotFound),
            (Some(_), Some(_)) => Err(FindUniqueMatchError::NotUnique),
        }
    }

    /// Find an item that matches `predicate` but only if no other item matches.
    ///
    /// - If a unique match is found, returns `Ok(Some(item))`
    /// - If a no match is found, returns `Ok(None)`
    /// - If a non-unique match is found, returns `Err(FindUniqueMatchError::NotUnique)`
    fn try_find_unique<P>(
        &mut self,
        predicate: &P,
    ) -> Result<Option<Self::Item>, FindUniqueMatchError>
    where
        Self: Sized,
        P: Fn(&Self::Item) -> bool,
    {
        match (self.find(predicate), self.find(predicate)) {
            (Some(column), None) => Ok(Some(column)),
            (None, _) => Ok(None),
            (Some(_), Some(_)) => Err(FindUniqueMatchError::NotUnique),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum FindUniqueMatchError {
    #[error("matched more than one item")]
    NotUnique,

    #[error("no match found")]
    NotFound,
}

impl<T: Iterator> IteratorExt for T {}
