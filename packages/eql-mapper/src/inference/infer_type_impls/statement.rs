use sqlparser::ast::{AssignmentTarget, Statement};

use crate::{
    inference::infer_type::InferType, inference::unifier::Type, TypeError, TypeInferencer,
};

impl<'ast> InferType<'ast, Statement> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, statement: &'ast Statement) -> Result<(), TypeError> {
        match statement {
            Statement::Query(query) => {
                self.unify_and_log(statement, self.get_type(statement), self.get_type(&**query))?;
            }

            Statement::Insert(insert) => {
                self.unify_and_log(statement, self.get_type(statement), self.get_type(insert))?;
            }

            Statement::Delete(delete) => {
                self.unify_and_log(statement, self.get_type(statement), self.get_type(delete))?;
            }

            Statement::Update {
                // FIXME: use table to resolve the assignments (instead of looking up the columns names in the scope).
                table: _,
                assignments,
                returning,
                ..
            } => {
                for assignment in assignments.iter() {
                    match &assignment.target {
                        AssignmentTarget::ColumnName(object_name) => {
                            let ty = self
                                .scope
                                .borrow()
                                .resolve_ident(object_name.0.last().unwrap())?;

                            self.unify_and_log(assignment, ty, self.get_type(&assignment.value))?;
                        }

                        AssignmentTarget::Tuple(_) => {
                            return Err(TypeError::UnsupportedSqlFeature(
                                "tuple assignment target in UPDATE".into(),
                            ))
                        }
                    }
                }

                match returning {
                    Some(returning) => {
                        self.unify_and_log(
                            statement,
                            self.get_type(statement),
                            self.get_type(returning),
                        )?;
                    }
                    None => {
                        self.unify_and_log(statement, self.get_type(statement), Type::empty())?;
                    }
                }
            }

            Statement::Merge {
                into: _,
                table: _,
                source: _,
                on: _,
                clauses: _,
            } => {
                todo!()
            }

            _ => {}
        }

        Ok(())
    }
}
