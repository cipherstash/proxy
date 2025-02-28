use sqlparser::ast::SetExpr;

use crate::{inference::type_error::TypeError, inference::InferType, TypeInferencer};

impl<'ast> InferType<'ast, SetExpr> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, set_expr: &'ast SetExpr) -> Result<(), TypeError> {
        match set_expr {
            SetExpr::Select(select) => {
                self.unify(self.get_type(set_expr), self.get_type(&**select))?;
            }

            SetExpr::Query(query) => {
                self.unify(self.get_type(set_expr), self.get_type(&**query))?;
            }

            SetExpr::SetOperation {
                op: _,
                set_quantifier: _,
                left,
                right,
            } => {
                self.unify(
                    self.get_type(set_expr),
                    self.unify(self.get_type(&**left), self.get_type(&**right))?,
                )?;
            }

            SetExpr::Values(values) => {
                self.unify(self.get_type(values), self.get_type(set_expr))?;
            }

            SetExpr::Insert(statement) => {
                self.unify(self.get_type(statement), self.get_type(set_expr))?;
            }

            SetExpr::Update(statement) => {
                self.unify(self.get_type(statement), self.get_type(set_expr))?;
            }

            SetExpr::Table(table) => {
                self.unify(self.get_type(&**table), self.get_type(set_expr))?;
            }
        }

        Ok(())
    }
}
