use crate::eql::Identifier;
use crate::error::{EncryptError, Error, MappingError};
use crate::log::MAPPER;
use crate::postgresql::context::column::Column;
use crate::prometheus::STATEMENTS_UNMAPPABLE_TOTAL;
use cipherstash_client::schema::ColumnConfig;
use eql_mapper::{EqlMapperError, EqlTerm, TableColumn, TypeCheckedStatement};
use eql_mapper::TableResolver;
use metrics::counter;
use postgres_types::Type;
use sqltk::parser::ast;
use std::sync::Arc;
use tracing::{debug, warn};

pub struct StatementAnalyzer<'a> {
    typed_statement: TypeCheckedStatement<'a>,
}

impl<'a> StatementAnalyzer<'a> {
    /// Creates a new StatementAnalyzer by type-checking the provided statement
    ///
    /// Performs type checking against the database schema and creates an analyzer
    /// that holds the typed statement for subsequent column analysis operations.
    ///
    /// # Arguments
    ///
    /// * `statement` - The parsed AST statement to analyze
    /// * `table_resolver` - Database schema resolver for type checking
    /// * `mapping_errors_enabled` - Whether to include mapping error details in logs
    ///
    /// # Returns
    ///
    /// Returns a StatementAnalyzer instance on successful type checking, or an Error
    /// if the statement cannot be type-checked against the schema.
    pub fn new(
        statement: &'a ast::Statement,
        table_resolver: Arc<TableResolver>,
        mapping_errors_enabled: bool,
    ) -> Result<Self, Error> {
        match eql_mapper::type_check(table_resolver, statement) {
            Ok(typed_statement) => {
                debug!(target: MAPPER,
                    typed_statement = ?typed_statement
                );

                Ok(Self { typed_statement })
            }
            Err(EqlMapperError::InternalError(str)) => {
                warn!(
                    msg = "Internal Error in EQL Mapper",
                    mapping_errors_enabled,
                    error = str,
                );
                counter!(STATEMENTS_UNMAPPABLE_TOTAL).increment(1);
                Err(MappingError::Internal(str).into())
            }
            Err(err) => {
                warn!(
                    msg = "Unmappable statement",
                    mapping_errors_enabled,
                    error = err.to_string(),
                );
                counter!(STATEMENTS_UNMAPPABLE_TOTAL).increment(1);
                Err(MappingError::StatementCouldNotBeTypeChecked(err.to_string()).into())
            }
        }
    }

    /// Proxy methods for TypeCheckedStatement functionality
    pub fn requires_transform(&self) -> bool {
        self.typed_statement.requires_transform()
    }

    pub fn literal_values(&self) -> &Vec<(EqlTerm, &ast::Value)> {
        self.typed_statement.literal_values()
    }

    pub fn transform(&self, encrypted_nodes: std::collections::HashMap<sqltk::NodeKey, sqltk::parser::ast::Value>) -> Result<sqltk::parser::ast::Statement, eql_mapper::EqlMapperError> {
        self.typed_statement.transform(encrypted_nodes)
    }

    pub fn literals(&self) -> &Vec<(EqlTerm, &ast::Value)> {
        &self.typed_statement.literals
    }

    /// Maps typed statement projection columns to an Encrypt column configuration
    ///
    /// The returned `Vec` is of `Option<Column>` because the Projection columns are a mix of native and EQL types.
    /// Only EQL colunms will have a configuration. Native types are always None.
    ///
    /// Preserves the ordering and semantics of the projection to reduce the complexity of positional encryption.
    pub fn get_projection_columns(
        &self,
        get_column_config: impl Fn(&Identifier) -> Option<ColumnConfig>,
    ) -> Result<Vec<Option<Column>>, Error> {
        let mut projection_columns = vec![];

        for col in self.typed_statement.projection.columns() {
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
                    Self::get_column(identifier, eql_term, &get_column_config)?
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
        get_column_config: impl Fn(&Identifier) -> Option<ColumnConfig>,
    ) -> Result<Vec<Option<Column>>, Error> {
        let mut param_columns = vec![];

        for param in self.typed_statement.params.iter() {
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

                    Self::get_column(identifier, eql_term, &get_column_config)?
                }
                _ => None,
            };
            param_columns.push(configured_column);
        }

        Ok(param_columns)
    }

    pub fn get_literal_columns(
        &self,
        get_column_config: impl Fn(&Identifier) -> Option<ColumnConfig>,
    ) -> Result<Vec<Option<Column>>, Error> {
        let mut literal_columns = vec![];

        for (eql_term, _) in self.typed_statement.literals.iter() {
            let TableColumn { table, column } = eql_term.table_column();
            let identifier = Identifier::from((table, column));

            debug!(
                target: MAPPER,
                msg = "Encrypted literal",
                column = ?identifier,
                ?eql_term,
            );
            let col = Self::get_column(identifier, eql_term, &get_column_config)?;
            if col.is_some() {
                literal_columns.push(col);
            }
        }

        Ok(literal_columns)
    }

    /// Get the column configuration for the Identifier
    /// Returns `EncryptError::UnknownColumn` if configuration cannot be found for the Identified column
    /// if mapping enabled, and None if mapping is disabled. It'll log a warning either way.
    pub fn get_column(
        identifier: Identifier,
        eql_term: &EqlTerm,
        get_column_config: impl Fn(&Identifier) -> Option<ColumnConfig>,
    ) -> Result<Option<Column>, Error> {
        match get_column_config(&identifier) {
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
