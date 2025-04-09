use sqlparser::ast::Query;

use crate::{
    inference::{InferType, TypeError},
    unifier::{Constructor, Projection, ProjectionColumns, Type},
    TypeInferencer,
};

impl<'ast> InferType<'ast, Query> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, query: &'ast Query) -> Result<(), TypeError> {
        let Query { body, .. } = query;

        self.unify_nodes(query, &**body)?;

        self.resolve_value_types_in_select_statement_projection(query)?;

        Ok(())
    }
}

impl<'ast> TypeInferencer<'ast> {
    /// When an expression type in the outermost projection of a `SELECT` is [`Type::Var`] and if there exists an
    /// [`Expr::Value`] node in the AST with the same `Type::Var` then this function resolves the literal node to
    /// `Type::Constructor(Constructor::Value(Value::Native(NativeType(None))))`.
    ///
    /// # Background
    ///
    /// Literal expressions (i.e. [`Expr::Value`]) are assigned [`Type::Var`] initially. The unifier can only refine
    /// its type when it used in another expression. Without this function, unused literals would remain unresolved
    /// and the type check would fail.
    ///
    /// This function therefore unifies those unresolved types as "native". It is safe to do so because the unifier
    /// has already proved
    fn resolve_value_types_in_select_statement_projection(
        &mut self,
        query: &'ast Query,
    ) -> Result<(), TypeError> {
        let ty = self.get_type(query);
        if let Type::Constructor(Constructor::Projection(Projection::WithColumns(
                ProjectionColumns(cols),
            ))) = &*ty.as_type() {
            for col in cols {
                if let Type::Var(tvar) = &*col.ty.as_type() {
                    if self
                        .value_tracker
                        .borrow()
                        .exists_value_with_type_var(*tvar)
                    {
                        self.unify(col.ty.clone(), Type::any_native())?;
                    }
                }
            }
        };
        Ok(())
    }
}
