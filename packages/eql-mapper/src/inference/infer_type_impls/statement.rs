use std::sync::Arc;

use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{AssignmentTarget, ObjectName, ObjectNamePart, Statement, TableFactor};

use crate::{
    inference::infer_type::InferType,
    unifier::{EqlTerm, EqlValue, NativeValue, Type, Value},
    ColumnKind, TableColumn, TypeError, TypeInferencer,
};

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
                table,
                assignments,
                returning,
                ..
            } => {
                // Assignment targets belong to the table being updated, so
                // resolve them against `table` directly. Resolving through the
                // lexical scope would also see every `FROM`-joined relation,
                // letting a same-named column there shadow the target column
                // (or make it spuriously ambiguous).
                let target_table = match &table.relation {
                    TableFactor::Table { name, .. } if table.joins.is_empty() => name,
                    _ => {
                        return Err(TypeError::UnsupportedSqlFeature(
                            "UPDATE target that is not a plain table".into(),
                        ))
                    }
                };

                for assignment in assignments.iter() {
                    match &assignment.target {
                        AssignmentTarget::ColumnName(ObjectName(parts)) if parts.len() == 1 => {
                            let ObjectNamePart::Identifier(ident) = parts.last().unwrap();
                            let stc = self
                                .table_resolver
                                .resolve_table_column(target_table, ident)?;

                            let tc = TableColumn {
                                table: stc.table.clone(),
                                column: stc.column.clone(),
                            };

                            let value_ty = match &stc.kind {
                                ColumnKind::Native => Value::Native(NativeValue(Some(tc))),
                                ColumnKind::Eql(features, identity) => Value::Eql(EqlTerm::Full(
                                    EqlValue(tc, identity.clone(), *features),
                                )),
                                // An UPDATE assignment is a write path: there is
                                // no way to encrypt the incoming value, so
                                // accepting it would store plaintext. (CIP-3688)
                                ColumnKind::UnmappableEncrypted(column_type) => {
                                    return Err(TypeError::UnmappableEncryptedColumn {
                                        table: stc.table.value.clone(),
                                        // `.value` rather than `.to_string()` —
                                        // see the matching note in `Projection`.
                                        column: stc.column.value.clone(),
                                        column_type: column_type.clone(),
                                    })
                                }
                            };

                            self.unify_node_with_type(
                                &assignment.value,
                                Arc::new(Type::Value(value_ty)),
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
                // Note: inner statement's type inference happens through normal AST traversal
                statement: _inner_statement,
                ..
            } => {
                // Recursively type-check the inner statement so transformations apply
                // EXPLAIN itself returns metadata, not the query results - give it empty projection
                self.unify_node_with_type(statement, Type::empty_projection())?;
            }

            // Invariant: every statement variant admitted by
            // `requires_type_check` (see `eql_mapper.rs`) has an explicit arm
            // above that constrains the statement's top-level type. This arm
            // fails closed so that widening `requires_type_check` without
            // adding a matching arm becomes a loud error instead of a
            // silently-unconstrained statement.
            unhandled => {
                return Err(TypeError::InternalError(format!(
                    "type inference has no rule for statement `{unhandled}`; \
                     `requires_type_check` admits a statement variant that \
                     `InferType<'_, Statement>` does not handle"
                )))
            }
        };

        Ok(())
    }
}
