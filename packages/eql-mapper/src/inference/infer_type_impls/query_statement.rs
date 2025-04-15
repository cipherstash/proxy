use sqlparser::ast::{Query, Value};

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
    /// Literal expressions (i.e. [`Value`]) are assigned [`Type::Var`] initially. The unifier can only refine
    /// its type when it used in another expression.
    ///
    /// If the literal is *not* used in another expression that constrains its type and an expression with the same type
    /// variable as the literal is returned as a projection column then type checking would fail because of the
    /// post-typecheck invariant that there can be no unresolved type variables remaining.
    ///
    /// This function resolves unresolved type variables as `Native` when these conditions are met:
    ///
    /// 1. the type variable is the type of one or more projection columns of a `SELECT` statement.
    /// 2. there exists a `Value` node in the AST which is assigned the same type variable.
    fn resolve_value_types_in_select_statement_projection(
        &mut self,
        query: &'ast Query,
    ) -> Result<(), TypeError> {
        let ty = self.get_type(query);
        if let Type::Constructor(Constructor::Projection(Projection::WithColumns(
            ProjectionColumns(cols),
        ))) = &*ty.as_type()
        {
            for col in cols {
                if let Type::Var(tvar) = &*col.ty.as_type() {
                    if let Some((_, ty)) = self
                        .reg
                        .borrow()
                        .first_matching_node_with_type::<Value>(&Type::Var(*tvar))
                    {
                        let unified = self.unify(col.ty.clone(), ty)?;
                        self.unify(unified, Type::any_native())?;
                    }
                }
            }
        };
        Ok(())
    }
}
