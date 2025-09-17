use crate::{
    eql::Identifier,
    error::{EncryptError, Error},
    log::MAPPER,
    postgresql::Column,
    proxy::EncryptConfig,
};
use eql_mapper::{EqlTerm, TableColumn, TypeCheckedStatement};
use postgres_types::Type;
use std::sync::Arc;
use tracing::{debug, warn};

/// Service responsible for processing columns from type-checked SQL statements
/// and mapping them to encryption configurations.
#[derive(Clone)]
pub struct ColumnMapper {
    encrypt_config: Arc<EncryptConfig>,
}

impl ColumnMapper {
    /// Create a new ColumnProcessor with the given schema service and client ID
    pub fn new(encrypt_config: Arc<EncryptConfig>) -> Self {
        Self { encrypt_config }
    }

    /// Maps typed statement projection columns to an Encrypt column configuration
    ///
    /// The returned `Vec` is of `Option<Column>` because the Projection columns are a mix of native and EQL types.
    /// Only EQL columns will have a configuration. Native types are always None.
    ///
    /// Preserves the ordering and semantics of the projection to reduce the complexity of positional encryption.
    pub fn get_projection_columns(
        &self,
        typed_statement: &TypeCheckedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        let mut projection_columns = vec![];

        for col in typed_statement.projection.columns() {
            let eql_mapper::ProjectionColumn { ty, .. } = col;
            let configured_column = match &**ty {
                eql_mapper::Type::Value(eql_mapper::Value::Eql(eql_term)) => {
                    let TableColumn { table, column } = eql_term.table_column();
                    let identifier: Identifier = Identifier::from((table, column));

                    debug!(
                        target: MAPPER,
                        msg = "Configured column",
                        column = ?identifier,
                        ?eql_term,
                    );
                    self.get_column(identifier, eql_term)?
                }
                _ => None,
            };
            projection_columns.push(configured_column)
        }

        Ok(projection_columns)
    }

    /// Maps typed statement param columns to an Encrypt column configuration
    ///
    /// The returned `Vec` is of `Option<Column>` because the Param columns are a mix of native and EQL types.
    /// Only EQL colunms will have a configuration. Native types are always None.
    ///
    /// Preserves the ordering and semantics of the projection to reduce the complexity of positional encryption.
    pub fn get_param_columns(
        &self,
        typed_statement: &TypeCheckedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        let mut param_columns = vec![];

        for param in typed_statement.params.iter() {
            let configured_column = match param {
                (_, eql_mapper::Value::Eql(eql_term)) => {
                    let TableColumn { table, column } = eql_term.table_column();
                    let identifier = Identifier::from((table, column));

                    debug!(
                        target: MAPPER,
                        msg = "Encrypted parameter",
                        column = ?identifier,
                        ?eql_term,
                    );

                    self.get_column(identifier, eql_term)?
                }
                _ => None,
            };
            param_columns.push(configured_column);
        }

        Ok(param_columns)
    }

    /// Maps typed statement literal columns to an Encrypt column configuration
    pub fn get_literal_columns(
        &self,
        typed_statement: &TypeCheckedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        let mut literal_columns = vec![];

        for (eql_term, _) in typed_statement.literals.iter() {
            let TableColumn { table, column } = eql_term.table_column();
            let identifier = Identifier::from((table, column));

            debug!(
                target: MAPPER,
                msg = "Encrypted literal",
                column = ?identifier,
                ?eql_term,
            );
            let col = self.get_column(identifier, eql_term)?;
            if col.is_some() {
                literal_columns.push(col);
            }
        }

        Ok(literal_columns)
    }

    /// Get the column configuration for the Identifier
    /// Returns `EncryptError::UnknownColumn` if configuration cannot be found for the Identified column
    /// if mapping enabled, and None if mapping is disabled. It'll log a warning either way.
    fn get_column(
        &self,
        identifier: Identifier,
        eql_term: &EqlTerm,
    ) -> Result<Option<Column>, Error> {
        match self.encrypt_config.get_column_config(&identifier) {
            Some(config) => {
                debug!(
                    target: MAPPER,
                    msg = "Configured column",
                    column = ?identifier
                );

                // IndexTerm::SteVecSelector
                let postgres_type = if matches!(eql_term, EqlTerm::JsonPath(_)) {
                    Some(Type::JSONPATH)
                } else {
                    None
                };

                let eql_term = eql_term.variant();
                Ok(Some(Column::new(
                    identifier,
                    config,
                    postgres_type,
                    eql_term,
                )))
            }
            None => {
                warn!(
                    target: MAPPER,
                    msg = "Configured column not found. Encryption configuration may have been deleted.",
                    ?identifier,
                );
                Err(EncryptError::UnknownColumn {
                    table: identifier.table.to_owned(),
                    column: identifier.column.to_owned(),
                }
                .into())
            }
        }
    }
}
