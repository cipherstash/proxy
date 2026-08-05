use std::sync::Arc;

use crate::{
    inference::{type_error::TypeError, unifier::Type, InferType},
    unifier::{EqlTerm, EqlValue, NativeValue, Value},
    ColumnKind, SchemaTableColumn, TableColumn, TypeInferencer,
};
use eql_mapper_macros::trace_infer;
use sqltk::parser::ast::{
    AssignmentTarget, ConflictTarget, Insert, ObjectName, ObjectNamePart, OnConflict,
    OnConflictAction, OnInsert, TableObject,
};

/// The type of the value stored in a schema column: the column's EQL type when
/// it is encrypted, its native identity otherwise.
///
/// Naming a column as a write target (an INSERT column list, or an
/// `ON CONFLICT DO UPDATE` assignment) is the path the unmappable-column
/// refusal exists for: there is no way to encrypt the incoming value, so
/// accepting it would store plaintext. (CIP-3688)
fn stored_value_type(stc: &SchemaTableColumn) -> Result<(Value, TableColumn), TypeError> {
    let tc = TableColumn {
        table: stc.table.clone(),
        column: stc.column.clone(),
    };

    let value_ty = match &stc.kind {
        ColumnKind::Native => Value::Native(NativeValue(Some(tc.clone()))),
        ColumnKind::Eql(features, identity) => Value::Eql(EqlTerm::Full(EqlValue(
            tc.clone(),
            identity.clone(),
            *features,
        ))),
        ColumnKind::UnmappableEncrypted(column_type) => {
            return Err(TypeError::UnmappableEncryptedColumn {
                table: stc.table.value.clone(),
                // `.value` rather than `.to_string()` — see the matching note
                // in `Projection`.
                column: stc.column.value.clone(),
                column_type: column_type.clone(),
            });
        }
    };

    Ok((value_ty, tc))
}

#[trace_infer]
impl<'ast> InferType<'ast, Insert> for TypeInferencer<'ast> {
    fn infer_enter(&mut self, insert: &'ast Insert) -> Result<(), TypeError> {
        if let Insert {
            table: TableObject::TableName(table_name),
            table_alias,
            columns,
            source,
            assignments,
            on,
            ..
        } = insert
        {
            if table_alias.is_some() {
                return Err(TypeError::UnsupportedSqlFeature("INSERT with ALIAS".into()));
            }

            if !assignments.is_empty() {
                return Err(TypeError::UnsupportedSqlFeature(
                    "MySQL INSERT ... SET".into(),
                ));
            }

            let table_columns = if columns.is_empty() {
                // When no columns are specified, the source must unify with a projection of ALL table columns.
                self.table_resolver.resolve_table_columns(table_name)?
            } else {
                columns
                    .iter()
                    .map(|c| self.table_resolver.resolve_table_column(table_name, c))
                    .collect::<Result<Vec<_>, _>>()?
            };

            let target_columns = Type::projection(
                &table_columns
                    .into_iter()
                    .map(|stc| {
                        let (value_ty, tc) = stored_value_type(&stc)?;
                        Ok((Arc::new(Type::Value(value_ty)), Some(tc.column)))
                    })
                    .collect::<Result<Vec<_>, TypeError>>()?,
            );

            if let Some(source) = source {
                self.unify_node_with_type(&**source, target_columns)?;
            }

            match on {
                Some(OnInsert::OnConflict(on_conflict)) => {
                    self.infer_on_conflict(table_name, on_conflict)?;
                }

                Some(OnInsert::DuplicateKeyUpdate(_)) => {
                    return Err(TypeError::UnsupportedSqlFeature(
                        "MySQL ON DUPLICATE KEY UPDATE".into(),
                    ));
                }

                // `OnInsert` is non-exhaustive: reject any variant this crate
                // does not know rather than let its assignments escape
                // inference.
                Some(_) => {
                    return Err(TypeError::UnsupportedSqlFeature(
                        "unrecognised ON clause in INSERT".into(),
                    ));
                }

                None => {}
            }

            Ok(())
        } else {
            Err(TypeError::UnsupportedSqlFeature("table functions".into()))
        }
    }

    fn infer_exit(&mut self, insert: &'ast Insert) -> Result<(), TypeError> {
        let Insert { returning, .. } = insert;

        match returning {
            Some(returning) => {
                self.unify_nodes(insert, returning)?;
            }

            None => {
                self.unify_node_with_type(insert, Type::empty_projection())?;
            }
        }

        Ok(())
    }
}

impl<'ast> TypeInferencer<'ast> {
    /// Constrains an `ON CONFLICT` clause of an `INSERT` against the target
    /// table.
    ///
    /// The `DO UPDATE SET` assignments are the upsert path of the statement:
    /// each assignment value is unified with the type stored in its target
    /// column, exactly as a plain `UPDATE ... SET` is — so a plaintext literal
    /// assigned to an encrypted column becomes an EQL literal to encrypt, and
    /// never lands in the column unencrypted.
    ///
    /// Assignment targets are resolved against the target table directly (not
    /// the lexical scope): PostgreSQL forbids qualifying them, and the scope
    /// also contains the `excluded` pseudo-table, which projects the same
    /// column names. Value expressions and the `DO UPDATE ... WHERE` predicate
    /// are ordinary expressions resolved against the scope — which is how
    /// `excluded.<col>` references get their types (see
    /// [`crate::importer::Importer`]).
    ///
    /// A conflict target on an encrypted column is rejected: a conflict only
    /// ever fires off a unique index, and uniqueness of an encrypted column
    /// would be judged on the whole jsonb payload — whose ciphertext is
    /// randomised per row — so the conflict would never fire and every upsert
    /// would silently insert a duplicate.
    ///
    /// `ON CONFLICT ON CONSTRAINT <name>` is let through as written: the
    /// mapper's schema carries no constraint catalog to resolve the name
    /// against, and a constraint over an encrypted column is a schema-design
    /// error that exists independently of any statement.
    fn infer_on_conflict(
        &mut self,
        table_name: &ObjectName,
        on_conflict: &'ast OnConflict,
    ) -> Result<(), TypeError> {
        let OnConflict {
            conflict_target,
            action,
        } = on_conflict;

        if let Some(ConflictTarget::Columns(columns)) = conflict_target {
            for column in columns {
                let stc = self
                    .table_resolver
                    .resolve_table_column(table_name, column)?;
                if matches!(stc.kind, ColumnKind::Eql(..)) {
                    return Err(TypeError::UnsupportedSqlFeature(format!(
                        "ON CONFLICT on encrypted column {}.{}",
                        stc.table, stc.column
                    )));
                }
            }
        }

        match action {
            OnConflictAction::DoNothing => {}

            OnConflictAction::DoUpdate(do_update) => {
                for assignment in &do_update.assignments {
                    match &assignment.target {
                        AssignmentTarget::ColumnName(ObjectName(parts)) if parts.len() == 1 => {
                            let ObjectNamePart::Identifier(ident) = parts.last().unwrap();
                            let stc = self
                                .table_resolver
                                .resolve_table_column(table_name, ident)?;
                            let (value_ty, _) = stored_value_type(&stc)?;
                            self.unify_node_with_type(&assignment.value, Type::Value(value_ty))?;
                        }

                        AssignmentTarget::ColumnName(ObjectName(_)) => {
                            return Err(TypeError::UnsupportedSqlFeature(
                                "qualified column names in ON CONFLICT DO UPDATE".into(),
                            ));
                        }

                        AssignmentTarget::Tuple(_) => {
                            return Err(TypeError::UnsupportedSqlFeature(
                                "tuple assignment target in ON CONFLICT DO UPDATE".into(),
                            ));
                        }
                    }
                }

                // `do_update.selection` needs no handling here: it is an
                // ordinary predicate whose identifiers resolve against the
                // scope (the target table and `excluded`), and whose operators
                // are constrained by the expression rules.
            }
        }

        Ok(())
    }
}
