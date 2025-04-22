use eql_mapper_macros::trace_infer;
use sqltk_parser::ast::SetExpr;

use crate::{inference::type_error::TypeError, inference::InferType, TypeInferencer};

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
                op: _,
                set_quantifier: _,
                left,
                right,
            } => {
                self.unify_node_with_type(set_expr, self.unify_nodes(&**left, &**right)?)?;
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
        }

        Ok(())
    }
}
