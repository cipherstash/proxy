use crate::{
    config::TandemConfig,
    connect, eql,
    error::Error,
    log::PROXY,
    postgresql::{Column, KeysetIdentifier},
    proxy::{config::EncryptConfigManager, schema::SchemaManager, zerokms::ZeroKms},
};
use cipherstash_client::{encryption::Plaintext, schema::ColumnConfig};
use tracing::{debug, warn};

mod config;
mod schema;
mod zerokms;

/// SQL Statement for loading encrypt configuration from database
const ENCRYPT_CONFIG_QUERY: &str = include_str!("./sql/select_config.sql");

/// SQL Statement for loading database schema
const SCHEMA_QUERY: &str = include_str!("./sql/select_table_schemas.sql");

/// SQL Statement for loading aggregates as part of database schema
const AGGREGATE_QUERY: &str = include_str!("./sql/select_aggregates.sql");

///
/// Core proxy service providing encryption, configuration, and schema management.
///
#[derive(Clone)]
pub struct Proxy {
    pub config: TandemConfig,
    pub encrypt_config: EncryptConfigManager,
    pub schema: SchemaManager,
    /// The EQL version installed in the database or `None` if it was not present
    pub eql_version: Option<String>,
    zerokms: ZeroKms,
}

impl Proxy {
    pub async fn init(config: TandemConfig) -> Result<Proxy, Error> {
        let zerokms = ZeroKms::init(&config)?;

        // Attempt to connect to default keyset
        // Ensures error on start if credential or network issue
        zerokms.init_cipher(None).await?;

        let encrypt_config = EncryptConfigManager::init(&config.database).await?;
        // TODO: populate EqlTraitImpls based in config
        let schema = SchemaManager::init(&config.database).await?;

        let eql_version = {
            let client = connect::database(&config.database).await?;
            let rows = client
                .query("SELECT eql_v2.version() AS version;", &[])
                .await;

            match rows {
                Ok(rows) => rows.first().map(|row| row.get("version")),
                Err(err) => {
                    warn!(
                        msg = "Could not query EQL version from database",
                        error = err.to_string()
                    );
                    None
                }
            }
        };

        Ok(Proxy {
            config,
            zerokms,
            encrypt_config,
            schema,
            eql_version,
        })
    }

    ///
    /// Encrypt `Plaintexts` using the `Column` configuration
    ///
    pub async fn encrypt(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        plaintexts: Vec<Option<Plaintext>>,
        columns: &[Option<Column>],
    ) -> Result<Vec<Option<eql::EqlEncrypted>>, Error> {
        debug!(target: PROXY, msg="Encrypt", ?keyset_id, default_keyset_id = ?self.config.encrypt.default_keyset_id);

        self.zerokms
            .encrypt(
                keyset_id,
                plaintexts,
                columns,
                self.config.encrypt.default_keyset_id,
            )
            .await
    }

    ///
    /// Decrypt eql::Ciphertext into Plaintext
    ///
    /// Database values are stored as `eql::Ciphertext`
    ///
    pub async fn decrypt(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        ciphertexts: Vec<Option<eql::EqlEncrypted>>,
    ) -> Result<Vec<Option<Plaintext>>, Error> {
        debug!(target: PROXY, msg="Decrypt", ?keyset_id, default_keyset_id = ?self.config.encrypt.default_keyset_id);

        self.zerokms
            .decrypt(
                keyset_id,
                ciphertexts,
                self.config.encrypt.default_keyset_id,
            )
            .await
    }

    pub fn get_column_config(&self, identifier: &eql::Identifier) -> Option<ColumnConfig> {
        let encrypt_config = self.encrypt_config.load();
        encrypt_config.get(identifier).cloned()
    }

    pub async fn reload_schema(&self) {
        self.schema.reload().await;
        self.encrypt_config.reload().await;
    }

    pub fn is_passthrough(&self) -> bool {
        self.encrypt_config.is_empty() || self.config.mapping_disabled()
    }

    pub fn is_empty_config(&self) -> bool {
        self.encrypt_config.is_empty()
    }
}

// Implement service traits for backward compatibility
#[async_trait::async_trait]
impl crate::services::EncryptionService for Proxy {
    async fn encrypt(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        plaintexts: Vec<Option<cipherstash_client::encryption::Plaintext>>,
        columns: &[Option<Column>],
    ) -> Result<Vec<Option<crate::EqlEncrypted>>, Error> {
        self.encrypt(keyset_id, plaintexts, columns).await
    }

    async fn decrypt(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        ciphertexts: Vec<Option<crate::EqlEncrypted>>,
    ) -> Result<Vec<Option<cipherstash_client::encryption::Plaintext>>, Error> {
        self.decrypt(keyset_id, ciphertexts).await
    }
}

#[async_trait::async_trait]
impl crate::services::SchemaService for Proxy {
    async fn reload_schema(&self) {
        self.reload_schema().await;
    }

    fn get_column_config(
        &self,
        identifier: &crate::eql::Identifier,
    ) -> Option<cipherstash_client::schema::ColumnConfig> {
        self.get_column_config(identifier)
    }

    fn get_table_resolver(&self) -> std::sync::Arc<eql_mapper::TableResolver> {
        self.schema.get_table_resolver()
    }

    fn is_passthrough(&self) -> bool {
        self.is_passthrough()
    }

    fn is_empty_config(&self) -> bool {
        self.is_empty_config()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::TandemConfig;
    use crate::test_helpers::with_no_cs_vars;
    use cts_common::WorkspaceId;

    fn build_tandem_config(env: Vec<(&str, Option<&str>)>) -> TandemConfig {
        with_no_cs_vars(|| {
            temp_env::with_vars(env, || {
                TandemConfig::build("tests/config/unknown.toml").unwrap()
            })
        })
    }

    fn default_env_vars() -> Vec<(&'static str, Option<&'static str>)> {
        vec![
            ("CS_DATABASE__USERNAME", Some("postgres")),
            ("CS_DATABASE__PASSWORD", Some("password")),
            ("CS_DATABASE__NAME", Some("db")),
            ("CS_DATABASE__HOST", Some("localhost")),
            ("CS_DATABASE__PORT", Some("5432")),
            ("CS_ENCRYPT__KEYSET_ID", Some("c50d8463-60e9-41a5-86cd-5782e03a503c")),
            ("CS_ENCRYPT__CLIENT_ID", Some("e40f1692-6bb7-4bbd-a552-4c0f155be073")),
            ("CS_ENCRYPT__CLIENT_KEY", Some("a4627031a16b7065726d75746174696f6e90090e0805000b0d0c0106040f0a0302076770325f66726f6da16b7065726d75746174696f6e9007060a0b02090d080c00040f0305010e6570325f746fa16b7065726d75746174696f6e900a0206090b04050c070f0e010d030800627033a16b7065726d75746174696f6e98210514181d0818200a18190b1112181809130f15181a0717181e000e0103181f0d181c1602040c181b1006")),
        ]
    }

    #[test]
    fn build_zerokms_config_with_crn() {
        with_no_cs_vars(|| {
            let mut env = default_env_vars();
            env.push(("CS_CLIENT_ACCESS_KEY", Some("client-access-key")));
            env.push((
                "CS_WORKSPACE_CRN",
                Some("crn:ap-southeast-2.aws:3KISDURL3ZCWYZ2O"),
            ));

            let tandem_config = build_tandem_config(env);

            let zerokms_config = zerokms::build_zerokms_config(&tandem_config).unwrap();

            assert_eq!(
                WorkspaceId::try_from("3KISDURL3ZCWYZ2O").unwrap(),
                zerokms_config.workspace_id()
            );

            assert!(zerokms_config
                .base_url()
                .to_string()
                .contains("ap-southeast-2.aws"));
        });
    }
}
