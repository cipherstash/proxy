use super::type_error::TypeError;

/// Trait for inferring the [`Type`] of an AST node.
///
/// This trait is implemented only on [`crate::TypeInferencer`] for each relevant `sqlparser` AST node.
///
/// Implementations must override one or both of [`InferType::infer_enter`] or [`InferType::infer_exit`] and invoke
/// methods on the `TypeInferencer` to set type unification constraints.
pub(crate) trait InferType<'ast, T> {
    /// Invoked when the `TypeInferencer`'s [`sqltk::Visitor`] implementation enters a node.
    ///
    /// No child nodes of the current node will have been visited at this point.
    ///
    /// Usually it makes more sense to implement [`InferType::infer_enter`] because more information will be available
    /// to make use of in type inference, however, the design of `sqlparser`'s AST makes that difficult in some cases.
    ///
    /// Returns `Ok(())` on success, `Err(TypeError)` on failure.
    fn infer_enter(&mut self, _: &'ast T) -> Result<(), TypeError> {
        Ok(())
    }

    /// Invoked when the `TypeInferencer`'s [`sqltk::Visitor`] implementation enters a node.
    ///
    /// Usually it make sense to only implement [`InferType::infer_exit`] because more information will be available due
    /// to child nodes of the current node already having been visited.
    ///
    /// Returns `Ok(())` on success, `Err(TypeError)` on failure.
    fn infer_exit(&mut self, _: &'ast T) -> Result<(), TypeError> {
        Ok(())
    }
}
