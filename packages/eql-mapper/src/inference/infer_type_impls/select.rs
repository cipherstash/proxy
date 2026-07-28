use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{Distinct, Expr, Select, SelectItem};

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
                    let (SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, .. }) =
                        item
                    else {
                        continue;
                    };

                    self.unify_node_with_eq_bound(expr)?;
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
}
