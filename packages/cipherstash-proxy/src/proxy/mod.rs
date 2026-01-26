use std::sync::Arc;

use crate::{
    config::TandemConfig,
    connect,
    error::Error,
    postgresql::{Column, Context, KeysetIdentifier},
    proxy::{encrypt_config::EncryptConfigManager, schema::SchemaManager},
};
use cipherstash_client::encryption::Plaintext;
use tokio::sync::mpsc::{self, UnboundedReceiver, UnboundedSender};
use tokio::sync::oneshot::Sender;
use tracing::{debug, warn};

mod encrypt_config;
mod schema;
mod zerokms;

pub use encrypt_config::EncryptConfig;
pub use zerokms::ZeroKms;

pub type ReloadSender = UnboundedSender<ReloadCommand>;

type ReloadReceiver = UnboundedReceiver<ReloadCommand>;

pub type ReloadResponder = Sender<()>;

/// SQL Statement for loading encrypt configuration from database
const ENCRYPT_CONFIG_QUERY: &str = include_str!("./sql/select_config.sql");

/// SQL Statement for loading database schema
const SCHEMA_QUERY: &str = include_str!("./sql/select_table_schemas.sql");

/// SQL Statement for loading aggregates as part of database schema
const AGGREGATE_QUERY: &str = include_str!("./sql/select_aggregates.sql");

#[derive(Debug)]
pub enum ReloadCommand {
    DatabaseSchema(ReloadResponder),
    EncryptSchema(ReloadResponder),
}

///
/// Core proxy service providing encryption, configuration, and schema management.
///
pub struct Proxy {
    pub config: Arc<TandemConfig>,
    pub encrypt_config_manager: EncryptConfigManager,
    pub schema_manager: SchemaManager,
    /// The EQL version installed in the database or `None` if it was not present
    pub eql_version: Option<String>,
    zerokms: ZeroKms,
    reload_sender: ReloadSender,
}

impl Proxy {
    pub async fn init(config: TandemConfig) -> Result<Proxy, Error> {
        let zerokms = ZeroKms::init(&config)?;

        // Attempt to connect to default keyset
        // Ensures error on start if credential or network issue
        zerokms.init_cipher(None).await?;

        let encrypt_config_manager = EncryptConfigManager::init(&config.database).await?;

        let schema_manager = SchemaManager::init(&config.database).await?;

        let eql_version = Proxy::eql_version(&config).await?;

        let (reload_sender, reload_receiver) = mpsc::unbounded_channel();

        Proxy::receive(
            reload_receiver,
            schema_manager.clone(),
            encrypt_config_manager.clone(),
        );

        Ok(Proxy {
            config: Arc::new(config),
            zerokms,
            encrypt_config_manager,
            schema_manager,
            eql_version,
            reload_sender,
        })
    }

    pub async fn eql_version(config: &TandemConfig) -> Result<Option<String>, Error> {
        let client = connect::database(&config.database).await?;
        let rows = client
            .query("SELECT eql_v2.version() AS version;", &[])
            .await;

        let version = match rows {
            Ok(rows) => rows.first().map(|row| row.get("version")),
            Err(err) => {
                warn!(
                    msg = "Could not query EQL version from database",
                    error = err.to_string()
                );
                None
            }
        };
        Ok(version)
    }

    pub fn receive(
        mut reload_receiver: ReloadReceiver,
        schema_manager: SchemaManager,
        encrypt_config_manager: EncryptConfigManager,
    ) {
        tokio::task::spawn(async move {
            while let Some(command) = reload_receiver.recv().await {
                debug!(msg = "ReloadCommand received", ?command);
                match command {
                    ReloadCommand::DatabaseSchema(responder) => {
                        schema_manager.reload().await;
                        encrypt_config_manager.reload().await;
                        let _ = responder.send(());
                    }
                    ReloadCommand::EncryptSchema(responder) => {
                        encrypt_config_manager.reload().await;
                        let _ = responder.send(());
                    }
                }
            }
        });
    }

    ///
    /// Create a new context from the Proxy settings
    ///
    pub fn context(&self, client_id: i32) -> Context<ZeroKms> {
        let config = self.config.clone();
        let encrypt_config = self.encrypt_config_manager.load();
        let schema = self.schema_manager.load();
        let reload_sender = self.reload_sender.clone();
        let encryption = self.zerokms.clone();

        Context::new(
            client_id,
            config,
            encrypt_config,
            schema,
            encryption,
            reload_sender,
        )
    }
}

#[async_trait::async_trait]
pub trait EncryptionService: Send + Sync {
    /// Encrypt plaintexts for storage in the database
    async fn encrypt(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        plaintexts: Vec<Option<Plaintext>>,
        columns: &[Option<Column>],
    ) -> Result<Vec<Option<crate::EqlCiphertext>>, Error>;

    /// Decrypt values retrieved from the database
    async fn decrypt(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        ciphertexts: Vec<Option<crate::EqlCiphertext>>,
    ) -> Result<Vec<Option<Plaintext>>, Error>;
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
                TandemConfig::build_path("tests/config/unknown.toml").unwrap()
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
