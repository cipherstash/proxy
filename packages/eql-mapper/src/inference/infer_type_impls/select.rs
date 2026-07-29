use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{Distinct, Expr, Select, SelectItem};

use crate::unifier::{Projection, Type, Value};
use crate::{
    inference::{type_error::TypeError, InferType},
    EqlTrait, TypeInferencer,
};

#[trace_infer]
impl<'ast> InferType<'ast, Select> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, select: &'ast Select) -> Result<(), TypeError> {
        self.unify_nodes(select, &select.projection)?;

        // Deduplication is equality, so every expression `DISTINCT` dedupes on
        // must support it. For an encrypted column that means its domain has to
        // carry an equality term — `eql_v3_boolean`, for instance, is
        // storage-only and cannot be deduplicated at all.
        //
        // Without this bound the requirement goes unchecked and the failure is
        // silent rather than loud: PostgreSQL would dedupe on the raw jsonb
        // payload, whose ciphertext is randomised per row, so every row looks
        // distinct and `DISTINCT` degrades into a no-op.
        match &select.distinct {
            Some(Distinct::Distinct) => {
                for item in &select.projection {
                    match item {
                        SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. } => {
                            self.unify_node_with_eq_bound(expr)?;
                        }

                        // A wildcard has no per-column expression to bind, but
                        // it still projects the columns `DISTINCT` dedupes on —
                        // and they are the ones most easily overlooked, being
                        // invisible in the query text. Check the columns the
                        // wildcard resolved to instead.
                        _ => self.check_projected_columns_support_eq(item)?,
                    }
                }
            }

            Some(Distinct::On(exprs)) => {
                for expr in exprs {
                    self.unify_node_with_eq_bound(expr)?;
                }
            }

            None => {}
        }

        Ok(())
    }
}

impl<'ast> TypeInferencer<'ast> {
    /// Constrains `node` to a type that implements [`EqlTrait::Eq`].
    ///
    /// A native type satisfies this trivially; the bound only bites for an
    /// encrypted column, whose domain must carry an equality term.
    fn unify_node_with_eq_bound(&mut self, node: &'ast Expr) -> Result<(), TypeError> {
        let bounded = self
            .unifier
            .borrow_mut()
            .fresh_bounded_tvar(EqlTrait::Eq.into());

        // Unify the node's *resolved* type with the bound, rather than just
        // pointing the node at a bounded variable: a variable satisfies any
        // bound vacuously, so binding alone would defer the check indefinitely
        // and never reject the column.
        let unified = self.unify(self.get_node_type(node), bounded)?;
        self.unify_node_with_type(node, unified)?;

        Ok(())
    }

    /// Requires every encrypted column a wildcard projects to implement
    /// [`EqlTrait::Eq`].
    ///
    /// The wildcard resolves to a projection type rather than to expressions, so
    /// there is no node to bind a variable to — the traits are read off the
    /// resolved columns directly.
    fn check_projected_columns_support_eq(
        &mut self,
        item: &'ast SelectItem,
    ) -> Result<(), TypeError> {
        let ty = self.get_node_type(item);

        let Type::Value(Value::Projection(projection)) = &*ty else {
            return Ok(());
        };

        Self::check_projection_supports_eq(projection)
    }

    /// Walks a projection, requiring every encrypted column in it to implement
    /// [`EqlTrait::Eq`].
    ///
    /// A wildcard's projection nests — one entry per relation in the `FROM`,
    /// each holding that relation's columns — so this recurses rather than
    /// looking only at the top level.
    fn check_projection_supports_eq(projection: &Projection) -> Result<(), TypeError> {
        for column in projection.columns() {
            match &*column.ty {
                Type::Value(Value::Projection(nested)) => {
                    Self::check_projection_supports_eq(nested)?
                }

                Type::Value(Value::Eql(eql_term)) if !eql_term.eql_value().trait_impls().eq => {
                    return Err(TypeError::UnsatisfiedBounds(
                        column.ty.clone(),
                        EqlTrait::Eq.into(),
                    ))
                }

                _ => {}
            }
        }

        Ok(())
    }
}
