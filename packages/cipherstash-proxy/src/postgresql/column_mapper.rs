use crate::{
    error::{EncryptError, Error},
    log::MAPPER,
    postgresql::Column,
    proxy::EncryptConfig,
};
use cipherstash_client::eql::Identifier;
use eql_mapper::{EqlTerm, ParamPlan, TableColumn, TypeCheckedStatement};
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
                    let identifier: Identifier =
                        Identifier::new(table.value.to_string(), column.value.to_string());

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
                    let identifier =
                        Identifier::new(table.value.to_string(), column.value.to_string());

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

    /// Maps the params of the *rewritten* statement to an Encrypt column
    /// configuration, positionally over [`ParamPlan::outputs`].
    ///
    /// These are the values actually sent to PostgreSQL, which after a fusion
    /// are not the values the client bound — hence a separate mapping from
    /// [`Self::get_param_columns`].
    pub fn get_output_param_columns(&self, plan: &ParamPlan) -> Result<Vec<Option<Column>>, Error> {
        let mut output_columns = vec![];

        for output in plan.outputs() {
            let configured_column = match &output.value {
                eql_mapper::Value::Eql(eql_term) => {
                    let TableColumn { table, column } = eql_term.table_column();
                    let identifier =
                        Identifier::new(table.value.to_string(), column.value.to_string());

                    debug!(
                        target: MAPPER,
                        msg = "Encrypted output parameter",
                        param = %output.param,
                        column = ?identifier,
                        ?eql_term,
                    );

                    self.get_column(identifier, eql_term)?
                }
                _ => None,
            };
            output_columns.push(configured_column);
        }

        Ok(output_columns)
    }

    /// Maps typed statement literal columns to an Encrypt column configuration
    pub fn get_literal_columns(
        &self,
        typed_statement: &TypeCheckedStatement<'_>,
    ) -> Result<Vec<Option<Column>>, Error> {
        let mut literal_columns = vec![];

        for (eql_term, _) in typed_statement.literals.iter() {
            let TableColumn { table, column } = eql_term.table_column();
            let identifier = Identifier::new(table.value.to_string(), column.value.to_string());

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
