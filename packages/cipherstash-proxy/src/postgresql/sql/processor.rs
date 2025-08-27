use crate::error::{EncryptError, Error};
use crate::postgresql::context::{Context, KeysetIdentifier};
use crate::prometheus::STATEMENTS_TOTAL;
use eql_mapper::{self, TableResolver};
use metrics::counter;
use sqltk::parser::ast;
use sqltk::parser::dialect::PostgreSqlDialect;
use sqltk::parser::parser::Parser;
use std::sync::Arc;
use tracing::{debug, info};

const DIALECT: PostgreSqlDialect = PostgreSqlDialect {};

/// SQL processing service that handles parsing, validation, and analysis of SQL statements.
pub struct SqlProcessor;

impl SqlProcessor {
    /// Parse a single SQL statement string into an AST.
    ///
    /// # Arguments
    ///
    /// * `statement` - SQL statement string to parse
    ///
    /// # Returns
    ///
    /// Returns the parsed AST statement or an error if parsing fails.
    pub fn parse_statement(statement: &str) -> Result<ast::Statement, Error> {
        let statement = Parser::new(&DIALECT)
            .try_with_sql(statement)?
            .parse_statement()?;

        debug!(
            target: "mapper",
            statement = %statement,
            "Parsed SQL statement"
        );

        counter!(STATEMENTS_TOTAL).increment(1);

        Ok(statement)
    }

    /// Parse a SQL string potentially containing multiple statements into parsed ASTs.
    ///
    /// # Arguments
    ///
    /// * `statements` - SQL string containing one or more statements separated by semicolons
    ///
    /// # Returns
    ///
    /// Returns a vector of parsed AST statements or an error if parsing fails.
    pub fn parse_statements(statements: &str) -> Result<Vec<ast::Statement>, Error> {
        let statements = Parser::new(&DIALECT)
            .try_with_sql(statements)?
            .parse_statements()?;

        debug!(
            target: "mapper",
            statements = ?statements,
            count = statements.len(),
            "Parsed multiple SQL statements"
        );

        counter!(STATEMENTS_TOTAL).increment(statements.len() as u64);

        Ok(statements)
    }

    /// Check if a statement contains DDL that would change the schema.
    ///
    /// # Arguments
    ///
    /// * `table_resolver` - Schema resolver for DDL detection
    /// * `statement` - AST statement to check
    ///
    /// # Returns
    ///
    /// Returns `true` if the statement contains DDL that changes schema, `false` otherwise.
    pub fn check_for_schema_change(
        table_resolver: Arc<TableResolver>,
        statement: &ast::Statement,
    ) -> bool {
        let schema_changed = eql_mapper::collect_ddl(table_resolver, statement);

        if schema_changed {
            debug!(
                target: "mapper",
                msg = "Schema change detected in statement"
            );
        }

        schema_changed
    }

    /// Handle `SET CIPHERSTASH KEYSET_*` statements.
    ///
    /// # Arguments
    ///
    /// * `statement` - AST statement to check for keyset commands
    /// * `context` - Session context for storing keyset information
    /// * `default_keyset_configured` - Whether a default keyset is configured
    ///
    /// # Returns
    ///
    /// Returns the keyset identifier if a SET command was processed, or an error
    /// if the command is invalid or conflicts with configuration.
    ///
    /// # Errors
    ///
    /// - Returns `EncryptError::UnexpectedSetKeyset` if called when a default keyset is configured
    /// - Returns parse errors if the keyset ID cannot be parsed as a valid UUID
    pub fn handle_set_keyset(
        statement: &ast::Statement,
        context: &mut Context,
        default_keyset_configured: bool,
    ) -> Result<Option<KeysetIdentifier>, Error> {
        if let Some(keyset_identifier) = context.maybe_set_keyset(statement)? {
            debug!(
                keyset_identifier = ?keyset_identifier,
                "Processing SET CIPHERSTASH.KEYSET command"
            );

            if default_keyset_configured {
                debug!(
                    target: "mapper",
                    keyset_identifier = ?keyset_identifier,
                    msg = "SET KEYSET command conflicts with configured default keyset"
                );
                return Err(EncryptError::UnexpectedSetKeyset.into());
            }

            info!(
                msg = "SET CIPHERSTASH.KEYSET",
                keyset_identifier = keyset_identifier.to_string()
            );

            return Ok(Some(keyset_identifier));
        }

        Ok(None)
    }
}