use crate::encrypt::Encrypt;
use crate::error::{Error, MappingError};
use crate::log::{CONTEXT, MAPPER};
use crate::postgresql::context::{column::Column, KeysetIdentifier, Statement};
use crate::postgresql::data::literal_from_sql;
use crate::postgresql::mapping::ColumnMapper;
use crate::postgresql::messages::bind::Bind;
use crate::prometheus::{
    ENCRYPTED_VALUES_TOTAL, ENCRYPTION_DURATION_SECONDS, ENCRYPTION_ERROR_TOTAL,
    ENCRYPTION_REQUESTS_TOTAL,
};
use crate::EqlEncrypted;
use cipherstash_client::encryption::Plaintext;
use eql_mapper::{EqlTerm, TypeCheckedStatement};
use metrics::{counter, histogram};
use sqltk::parser::ast;
use std::time::Instant;
use tracing::debug;

/// Encryption service for handling encrypted operations on SQL data.
///
/// Provides methods for encrypting parameter values, literal values, and
/// creating encryptable statements with proper column configurations.
pub struct EncryptionService<'a> {
    encrypt: &'a Encrypt,
    keyset_id: Option<KeysetIdentifier>,
}

impl<'a> EncryptionService<'a> {
    /// Create a new EncryptionService with access to encryption configuration.
    pub fn new(encrypt: &'a Encrypt, keyset_id: Option<KeysetIdentifier>) -> Self {
        Self { encrypt, keyset_id }
    }

    /// Encrypt literal values from a typed SQL statement.
    ///
    /// # Arguments
    ///
    /// * `typed_statement` - Type-checked statement containing literals
    /// * `literal_columns` - Column configurations for each literal
    ///
    /// # Returns
    ///
    /// Vector of encrypted values corresponding to each literal, with `None` for
    /// literals that don't require encryption and `Some(EqlEncrypted)` for encrypted values.
    pub async fn encrypt_literals(
        &self,
        typed_statement: &TypeCheckedStatement<'_>,
        literal_columns: &Vec<Option<Column>>,
    ) -> Result<Vec<Option<EqlEncrypted>>, Error> {
        let literal_values = typed_statement.literal_values();
        if literal_values.is_empty() {
            debug!(target: MAPPER,
                msg = "No literals to encrypt"
            );
            return Ok(vec![]);
        }

        let plaintexts = self.literals_to_plaintext(literal_values, literal_columns)?;

        let start = Instant::now();

        let encrypted = self
            .encrypt
            .encrypt(self.keyset_id.clone(), plaintexts, literal_columns)
            .await
            .inspect_err(|_| {
                counter!(ENCRYPTION_ERROR_TOTAL).increment(1);
            })?;

        debug!(target: MAPPER,
            ?literal_columns,
            ?encrypted,
            "Encrypted literals"
        );

        counter!(ENCRYPTION_REQUESTS_TOTAL).increment(1);
        counter!(ENCRYPTED_VALUES_TOTAL).increment(encrypted.len() as u64);

        let duration = Instant::now().duration_since(start);
        histogram!(ENCRYPTION_DURATION_SECONDS).record(duration);

        Ok(encrypted)
    }

    /// Encrypt parameter values for bind operations.
    ///
    /// # Arguments
    ///
    /// * `bind` - Bind message containing parameter values
    /// * `statement` - Statement containing column configurations and param types
    ///
    /// # Returns
    ///
    /// Vector of encrypted parameter values with `None` for non-encrypted params.
    ///
    /// # Notes
    ///
    /// Bind holds the params.
    /// Statement holds the column configuration and param types.
    ///
    /// Params are converted to plaintext using the column configuration and any `postgres_param_types` specified on Parse.
    pub async fn encrypt_params(
        &self,
        bind: &Bind,
        statement: &Statement,
    ) -> Result<Vec<Option<crate::EqlEncrypted>>, Error> {
        let plaintexts =
            bind.to_plaintext(&statement.param_columns, &statement.postgres_param_types)?;

        debug!(target: MAPPER, plaintexts = ?plaintexts);
        debug!(target: CONTEXT,
            keyset_id = ?self.keyset_id,
        );

        let start = Instant::now();

        let encrypted = self
            .encrypt
            .encrypt(self.keyset_id.clone(), plaintexts, &statement.param_columns)
            .await
            .inspect_err(|_| {
                counter!(ENCRYPTION_ERROR_TOTAL).increment(1);
            })?;

        // Avoid the iter calculation if we can
        if self.encrypt.config.prometheus_enabled() {
            let encrypted_count = encrypted.iter().filter(|e| e.is_some()).count() as u64;

            counter!(ENCRYPTION_REQUESTS_TOTAL).increment(1);
            counter!(ENCRYPTED_VALUES_TOTAL).increment(encrypted_count);

            let duration = Instant::now().duration_since(start);
            histogram!(ENCRYPTION_DURATION_SECONDS).record(duration);
        }

        Ok(encrypted)
    }

    /// Create an encryptable statement from a type-checked statement.
    ///
    /// # Arguments
    ///
    /// * `typed_statement` - Type-checked statement with EQL operations
    /// * `param_types` - PostgreSQL parameter types from Parse message
    ///
    /// # Returns
    ///
    /// Returns `Some(Statement)` if the statement contains encrypted operations,
    /// or `None` if no encryption is required.
    ///
    /// # Notes
    ///
    /// Creates a Statement from an EQL Mapper Typed Statement.
    /// Returned Statement contains the Column configuration for any encrypted columns in params, literals and projection.
    /// Returns `None` if the Statement is not Encryptable.
    pub fn to_encryptable_statement(
        &self,
        typed_statement: &TypeCheckedStatement<'_>,
        param_types: Vec<i32>,
    ) -> Result<Option<Statement>, Error> {
        let column_mapper = ColumnMapper::new(self.encrypt);
        let param_columns = column_mapper.get_param_columns(typed_statement)?;
        let projection_columns = column_mapper.get_projection_columns(typed_statement)?;
        let literal_columns = column_mapper.get_literal_columns(typed_statement)?;

        let no_encrypted_param_columns = param_columns.iter().all(|c| c.is_none());
        let no_encrypted_projection_columns = projection_columns.iter().all(|c| c.is_none());

        if (param_columns.is_empty() || no_encrypted_param_columns)
            && (projection_columns.is_empty() || no_encrypted_projection_columns)
            && !typed_statement.requires_transform()
        {
            return Ok(None);
        }

        debug!(target: MAPPER,
            msg = "Encryptable Statement",
            param_columns = ?param_columns,
            projection_columns = ?projection_columns,
            literal_columns = ?literal_columns,
        );

        let statement = Statement::new(
            param_columns.to_owned(),
            projection_columns.to_owned(),
            literal_columns.to_owned(),
            param_types,
        );

        Ok(Some(statement))
    }

    /// Convert SQL literal values to plaintext for encryption.
    ///
    /// # Arguments
    ///
    /// * `literals` - EQL terms and SQL values from the statement
    /// * `literal_columns` - Column configurations for each literal
    ///
    /// # Returns
    ///
    /// Vector of plaintext values with `None` for non-encrypted literals.
    ///
    /// # Errors
    ///
    /// Returns `MappingError::InvalidParameter` if literal conversion fails.
    fn literals_to_plaintext(
        &self,
        literals: &Vec<(EqlTerm, &ast::Value)>,
        literal_columns: &Vec<Option<Column>>,
    ) -> Result<Vec<Option<Plaintext>>, Error> {
        let plaintexts = literals
            .iter()
            .zip(literal_columns)
            .map(|((_, val), col)| match col {
                Some(col) => literal_from_sql(val, col.eql_term(), col.cast_type()).map_err(|err| {
                    debug!(
                        target: MAPPER,
                        msg = "Could not convert literal value",
                        value = ?val,
                        cast_type = ?col.cast_type(),
                        error = err.to_string()
                    );
                    MappingError::InvalidParameter(Box::new(col.to_owned()))
                }),
                None => Ok(None),
            })
            .collect::<Result<Vec<_>, _>>()?;
        Ok(plaintexts)
    }
}