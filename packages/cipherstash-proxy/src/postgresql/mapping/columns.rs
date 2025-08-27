use crate::encrypt::Encrypt;
use crate::error::{EncryptError, Error};
use crate::eql::Identifier;
use crate::log::MAPPER;
use crate::postgresql::context::column::Column;
use eql_mapper::{EqlTerm, TableColumn, TypeCheckedStatement};
use postgres_types::Type;
use tracing::{debug, warn};

/// Column mapping service that resolves EQL column configurations.
/// 
/// Maps typed statement columns (projection, parameters, literals) to encryption
/// column configurations for encrypted operations.
pub struct ColumnMapper<'a> {
    encrypt: &'a Encrypt,
}

impl<'a> ColumnMapper<'a> {
    /// Create a new ColumnMapper with access to encryption configuration.
    pub fn new(encrypt: &'a Encrypt) -> Self {
        Self { encrypt }
    }

    /// Maps typed statement projection columns to encryption column configurations.
    ///
    /// The returned `Vec` contains `Option<Column>` because projection columns are a mix 
    /// of native and EQL types. Only EQL columns will have a configuration. Native types are always None.
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
                        msg = "Configured projection column",
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

    /// Maps typed statement param columns to encryption column configurations.
    ///
    /// The returned `Vec` contains `Option<Column>` because param columns are a mix 
    /// of native and EQL types. Only EQL columns will have a configuration. Native types are always None.
    ///
    /// Preserves the ordering and semantics of the parameters to reduce the complexity of positional encryption.
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
                        msg = "Encrypted parameter column",
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

    /// Maps typed statement literal columns to encryption column configurations.
    ///
    /// Only returns configurations for literals that correspond to encrypted columns.
    /// Non-encrypted literals are filtered out since they don't need processing.
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
                msg = "Encrypted literal column",
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

    /// Get the column configuration for the given identifier and EQL term.
    ///
    /// # Arguments
    ///
    /// * `identifier` - Table and column identifier
    /// * `eql_term` - EQL term type information
    ///
    /// # Returns
    ///
    /// Returns `Some(Column)` if configuration exists, or an error if the column
    /// is expected to be encrypted but no configuration is found.
    ///
    /// # Errors
    ///
    /// Returns `EncryptError::UnknownColumn` if configuration cannot be found for 
    /// the identified column when mapping is enabled.
    pub fn get_column(
        &self,
        identifier: Identifier,
        eql_term: &EqlTerm,
    ) -> Result<Option<Column>, Error> {
        match self.encrypt.get_column_config(&identifier) {
            Some(config) => {
                debug!(
                    target: MAPPER,
                    msg = "Found column configuration",
                    column = ?identifier
                );

                // Special handling for JsonPath EQL terms
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
                    msg = "Column configuration not found. Encryption configuration may have been deleted.",
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