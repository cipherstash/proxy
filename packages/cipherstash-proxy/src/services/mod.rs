use crate::eql::Identifier;
use crate::error::Error;
use crate::postgresql::{Column, KeysetIdentifier};
use cipherstash_client::encryption::Plaintext;
use cipherstash_client::schema::ColumnConfig;
use eql_mapper::TableResolver;
use std::sync::Arc;

/// Service for handling encryption and decryption operations
#[async_trait::async_trait]
pub trait EncryptionService: Send + Sync {
    /// Encrypt plaintexts for storage in the database
    async fn encrypt(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        plaintexts: Vec<Option<Plaintext>>,
        columns: &[Option<Column>],
    ) -> Result<Vec<Option<crate::EqlEncrypted>>, Error>;

    /// Decrypt values retrieved from the database
    async fn decrypt(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        ciphertexts: Vec<Option<crate::EqlEncrypted>>,
    ) -> Result<Vec<Option<Plaintext>>, Error>;
}

/// Service for handling schema operations and column configuration
#[async_trait::async_trait]
pub trait SchemaService: Send + Sync {
    /// Reload the schema from the database
    async fn reload_schema(&self);

    /// Get column configuration for encryption
    fn get_column_config(&self, identifier: &Identifier) -> Option<ColumnConfig>;

    /// Get the table resolver for schema operations
    fn get_table_resolver(&self) -> Arc<TableResolver>;

    /// Check if the service is in passthrough mode
    fn is_passthrough(&self) -> bool;

    /// Check if the service has empty configuration
    fn is_empty_config(&self) -> bool;
}
