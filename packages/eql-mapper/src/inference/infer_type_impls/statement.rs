use sqlparser::ast::{AssignmentTarget, Statement};

use crate::{inference::infer_type::InferType, TypeError, TypeInferencer, TID};

impl<'ast> InferType<'ast, Statement> for TypeInferencer<'ast> {
    fn infer_exit(&mut self, statement: &'ast Statement) -> Result<(), TypeError> {
        match statement {
            Statement::Query(query) => {
                self.unify_nodes(statement, &**query)?;
            }

            Statement::Insert(insert) => {
                self.unify_nodes(statement, insert)?;
            }

            Statement::Delete(delete) => {
                self.unify_nodes(statement, delete)?;
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
                            self.unify_node_with_type(
                                &assignment.value,
                                self.scope_tracker
                                    .borrow()
                                    .resolve_ident(object_name.0.last().unwrap())?,
                            )?;
                        }

                        AssignmentTarget::Tuple(_) => {
                            return Err(TypeError::UnsupportedSqlFeature(
                                "tuple assignment target in UPDATE".into(),
                            ))
                        }
                    }
                }

                match returning {
                    Some(returning) => self.unify_nodes(statement, returning)?,
                    None => self.unify_node_with_type(statement, TID::EMPTY_PROJECTION)?,
                };
            }

            Statement::Merge {
                into: _,
                table: _,
                source: _,
                on: _,
                clauses: _,
            } => {
                return Err(TypeError::UnsupportedSqlFeature(
                    "MERGE is not yet supported".into(),
                ))
            }

            Statement::Prepare {
                name: _,
                data_types: _,
                statement: _,
            } => {
                return Err(TypeError::UnsupportedSqlFeature(
                    "PREPARE is not yet supported".into(),
                ))
            }

            _ => {}
        };

        Ok(())
    }
}
