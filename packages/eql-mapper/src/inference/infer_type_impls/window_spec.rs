use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{WindowFrameBound, WindowFrameUnits, WindowSpec};

use crate::{
    inference::infer_type::InferType,
    unifier::{Type, Value},
    EqlTrait, TypeError, TypeInferencer,
};

/// A window specification, wherever it is written: inline (`OVER (...)`) or as
/// a named window definition (`WINDOW w AS (...)`). Both forms contain a
/// [`WindowSpec`] node, so constraining the node itself covers `OVER w` too —
/// the named definition is checked once, at its definition site.
///
/// - Partitioning groups rows by equality, so every `PARTITION BY` key needs an
///   equality term. Without the bound, `PARTITION BY enc` partitions on the raw
///   jsonb payload — whose ciphertext is randomised per row — and every row
///   lands in a partition of one.
/// - The window's `ORDER BY` sorts the partition, so every key needs an
///   ordering term. Without the bound, `row_number() OVER (ORDER BY enc)`
///   numbers rows in ciphertext order, which differs on every insert.
/// - A frame offset (`ROWS 5 PRECEDING`) is a native scalar.
/// - A `RANGE` frame with an offset needs arithmetic (`key ± offset`) on the
///   sort key, and no term supports arithmetic — it is rejected when the key is
///   encrypted. `ROWS` and `GROUPS` frames need only ordering and peer
///   equality, which the (deterministic) ordering term preserves.
#[trace_infer]
impl<'ast> InferType<'ast, WindowSpec> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, spec: &'ast WindowSpec) -> Result<(), TypeError> {
        for expr in &spec.partition_by {
            self.unify_node_with_bound(expr, EqlTrait::Eq)?;
        }

        for order_by_expr in &spec.order_by {
            self.unify_node_with_bound(&order_by_expr.expr, EqlTrait::Ord)?;
        }

        if let Some(frame) = &spec.window_frame {
            let mut has_offset = false;

            let bounds = std::iter::once(&frame.start_bound).chain(frame.end_bound.as_ref());
            for bound in bounds {
                if let WindowFrameBound::Preceding(Some(offset))
                | WindowFrameBound::Following(Some(offset)) = bound
                {
                    has_offset = true;
                    self.unify_node_with_type(&**offset, Type::native())?;
                }
            }

            if has_offset && frame.units == WindowFrameUnits::Range {
                for order_by_expr in &spec.order_by {
                    let ty = self.get_node_type(&order_by_expr.expr);
                    let ty = ty.follow_tvars(&self.unifier.borrow());
                    if matches!(&*ty, Type::Value(Value::Eql(_))) {
                        return Err(TypeError::UnsupportedSqlFeature(
                            "RANGE window frame with an offset over an encrypted ORDER BY key"
                                .into(),
                        ));
                    }
                }
            }
        }

        Ok(())
    }
}
