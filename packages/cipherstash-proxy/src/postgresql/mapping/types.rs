use crate::error::{Error, MappingError};
use crate::log::MAPPER;
use crate::prometheus::STATEMENTS_UNMAPPABLE_TOTAL;
use eql_mapper::{self, EqlMapperError, TableResolver, TypeCheckedStatement};
use metrics::counter;
use sqltk::parser::ast;
use std::sync::Arc;
use tracing::{debug, warn};

/// Type checking service for SQL statements using EQL mapper.
///
/// Handles analysis of SQL statements to determine type compatibility
/// with encrypted operations and performs type inference for EQL columns.
pub struct TypeChecker {
    table_resolver: Arc<TableResolver>,
}

impl TypeChecker {
    /// Create a new TypeChecker with access to schema information.
    pub fn new(table_resolver: Arc<TableResolver>) -> Self {
        Self { table_resolver }
    }

    /// Check if a statement requires type checking for encrypted operations.
    ///
    /// # Arguments
    ///
    /// * `statement` - AST statement to analyze
    ///
    /// # Returns
    ///
    /// Returns `true` if the statement contains operations that require type checking.
    pub fn requires_type_check(&self, statement: &ast::Statement) -> bool {
        eql_mapper::requires_type_check(statement)
    }

    /// Perform type checking on a SQL statement for EQL operations.
    ///
    /// # Arguments
    ///
    /// * `statement` - AST statement to type check
    ///
    /// # Returns
    ///
    /// Returns a type-checked statement with inferred types for EQL operations,
    /// or an error if type checking fails.
    ///
    /// # Errors
    ///
    /// - Returns `MappingError::Internal` for internal EQL mapper errors
    /// - Returns `MappingError::StatementCouldNotBeTypeChecked` for type checking failures
    pub fn type_check<'a>(
        &self,
        statement: &'a ast::Statement,
    ) -> Result<TypeCheckedStatement<'a>, Error> {
        match eql_mapper::type_check(self.table_resolver.clone(), statement) {
            Ok(typed_statement) => {
                debug!(target: MAPPER,
                    typed_statement = ?typed_statement,
                    "Statement type checking completed"
                );

                Ok(typed_statement)
            }
            Err(EqlMapperError::InternalError(str)) => {
                warn!(
                    msg = "Internal Error in EQL Mapper",
                    error = str,
                );
                counter!(STATEMENTS_UNMAPPABLE_TOTAL).increment(1);
                Err(MappingError::Internal(str).into())
            }
            Err(err) => {
                warn!(
                    msg = "Unmappable statement",
                    error = err.to_string(),
                );
                counter!(STATEMENTS_UNMAPPABLE_TOTAL).increment(1);
                Err(MappingError::StatementCouldNotBeTypeChecked(err.to_string()).into())
            }
        }
    }
}