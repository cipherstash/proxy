use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{SetExpr, SetQuantifier};

use crate::{inference::type_error::TypeError, inference::InferType, TypeInferencer};

/// Whether a set operation with this quantifier removes duplicate rows.
///
/// `ALL` keeps them; everything else — including the absent quantifier, which
/// is plain `UNION` — deduplicates.
fn deduplicates(set_quantifier: &SetQuantifier) -> bool {
    !matches!(
        set_quantifier,
        SetQuantifier::All | SetQuantifier::AllByName
    )
}

#[trace_infer]
impl<'ast> InferType<'ast, SetExpr> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, set_expr: &'ast SetExpr) -> Result<(), TypeError> {
        match set_expr {
            SetExpr::Select(select) => {
                self.unify_nodes(set_expr, &**select)?;
            }

            SetExpr::Query(query) => {
                self.unify_nodes(set_expr, &**query)?;
            }

            SetExpr::SetOperation {
                op,
                set_quantifier,
                left,
                right,
            } => {
                let unified = self.unify_nodes(&**left, &**right)?;

                // A set operation without ALL deduplicates its rows, and it does
                // so through the type's default btree/hash operator class — not
                // through EQL's `=` overload. For a jsonb-backed domain that is
                // jsonb's own equality, which compares whole payloads including
                // `c`, the randomised ciphertext: no two rows ever match, so
                // UNION keeps every duplicate and INTERSECT/EXCEPT compare
                // nothing meaningfully.
                //
                // Unlike `SELECT DISTINCT`, this cannot be keyed on the
                // equality term in place — deduplication spans the whole
                // projection of both branches, and the terms would have to be
                // projected and then stripped back out. Refused rather than
                // silently wrong; `ALL` performs no deduplication and is
                // unaffected.
                if deduplicates(set_quantifier) && unified.contains_eql() {
                    return Err(TypeError::UnsupportedSqlFeature(format!(
                        "{op} on an encrypted column: deduplication would compare ciphertexts rather \
                         than values. Use {op} ALL, or deduplicate on a plaintext column."
                    )));
                }

                self.unify_node_with_type(set_expr, unified)?;
            }

            SetExpr::Values(values) => {
                self.unify_nodes(values, set_expr)?;
            }

            SetExpr::Insert(statement) => {
                self.unify_nodes(statement, set_expr)?;
            }

            SetExpr::Update(statement) => {
                self.unify_nodes(statement, set_expr)?;
            }

            SetExpr::Table(table) => {
                self.unify_nodes(&**table, set_expr)?;
            }

            SetExpr::Delete(statement) => {
                self.unify_nodes(statement, set_expr)?;
            }
        }

        Ok(())
    }
}
