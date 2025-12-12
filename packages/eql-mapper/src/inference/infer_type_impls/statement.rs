use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{AssignmentTarget, ObjectName, ObjectNamePart, Statement};

use crate::{inference::infer_type::InferType, unifier::Type, TypeError, TypeInferencer};

#[trace_infer]
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
                        AssignmentTarget::ColumnName(ObjectName(parts)) if parts.len() == 1 => {
                            let ObjectNamePart::Identifier(ident) = parts.last().unwrap();
                            self.unify_node_with_type(
                                &assignment.value,
                                self.resolve_ident(ident)?,
                            )?;
                        }

                        AssignmentTarget::ColumnName(ObjectName(_)) => {
                            return Err(TypeError::UnsupportedSqlFeature(
                                "qualified column names".into(),
                            ));
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
                    None => self.unify_node_with_type(statement, Type::empty_projection())?,
                };
            }

            Statement::Merge {
                into: _,
                table: _,
                source: _,
                on: _,
                clauses: _,
                output: _,
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

            Statement::Explain {
                statement: inner_statement,
                ..
            } => {
                // Recursively type-check the inner statement so transformations apply
                // EXPLAIN itself returns metadata, not the query results - give it empty projection
                self.unify_node_with_type(statement, Type::empty_projection())?;
                // Note: inner statement's type inference happens through normal AST traversal
                let _inner_statement = inner_statement; // Handled by visitor
            }

            _ => {}
        };

        Ok(())
    }
}
