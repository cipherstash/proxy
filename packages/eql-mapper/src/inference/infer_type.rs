use super::type_error::TypeError;

/// Trait for inferring the [`crate::Type`] of an AST node.
///
/// This trait is implemented only on [`crate::TypeInferencer`] for each relevant `sqlparser` AST node.
///
/// Implementations must override one or both of [`InferType::infer_enter`] or [`InferType::infer_exit`] and invoke
/// methods on the [`crate::TypeInferencer`] to set type unification constraints.
pub(crate) trait InferType<'ast, T> {
    /// Invoked when the `TypeInferencer` implementation enters a node.
    ///
    /// No child nodes of the current node will have been visited at this point.
    ///
    /// Returns `Ok(())` on success, `Err(TypeError)` on failure.
    #[allow(unused)]
    fn infer_enter(&mut self, node: &'ast T) -> Result<(), TypeError> {
        Ok(())
    }

    /// Invoked when the `TypeInferencer`'s [`sqltk::Visitor`] implementation exits a node.
    ///
    /// Returns `Ok(())` on success, `Err(TypeError)` on failure.
    #[allow(unused)]
    fn infer_exit(&mut self, node: &'ast T) -> Result<(), TypeError> {
        Ok(())
    }
}
