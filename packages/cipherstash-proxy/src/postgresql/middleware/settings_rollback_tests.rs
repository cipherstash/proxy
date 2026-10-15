//! Behavioral transaction-setting tests at the PostgreSQL message boundary.

use super::{Backend, Frontend};
use crate::postgresql::test_operation_id as operation_id;
use crate::{
    config::TandemConfig,
    error::{EncryptError, Error},
    postgresql::{
        context::{Context, KeysetIdentifier},
        Column,
    },
    proxy::{EncryptConfig, EncryptionService},
};
use bytes::Bytes;
use cipherstash_client::{encryption::Plaintext, IdentifiedBy};
use pg_proto::{BackendMessage, FrontendMessage, FrontendMiddlewareOutput, TransactionStatus};
use std::sync::{Arc, Mutex};

/// The external cipher records the keyset requested by an actual encrypted INSERT.
/// Returning an external-service failure avoids inventing cryptographic material.
#[derive(Clone, Default)]
struct Cipher {
    selected: Arc<Mutex<Vec<Option<KeysetIdentifier>>>>,
}

#[async_trait::async_trait]
impl EncryptionService for Cipher {
    async fn encrypt(
        &self,
        keyset: Option<KeysetIdentifier>,
        _: Vec<Option<Plaintext>>,
        _: &[Option<Column>],
    ) -> Result<Vec<Option<crate::EqlOutput>>, Error> {
        self.selected.lock().unwrap().push(keyset);
        Err(EncryptError::InvalidInboundEqlPayload.into())
    }
    async fn decrypt(
        &self,
        _: Option<KeysetIdentifier>,
        _: Vec<Option<crate::EqlCiphertext>>,
    ) -> Result<Vec<Option<Plaintext>>, Error> {
        Ok(Vec::new())
    }
    async fn decrypt_inbound_eql(
        &self,
        _: Option<KeysetIdentifier>,
        _: Vec<Option<crate::EqlCiphertext>>,
    ) -> Result<Vec<Option<Plaintext>>, Error> {
        Ok(Vec::new())
    }
}

struct Client {
    frontend: Frontend<Cipher>,
    backend: Backend<Cipher>,
    context: Context<Cipher>,
    cipher: Cipher,
}

impl Client {
    fn new() -> Self {
        Self::with_mapping_disabled(false)
    }

    fn with_mapping_disabled(disable_mapping: bool) -> Self {
        let mut config = TandemConfig::for_testing();
        config.encrypt.default_keyset_id = None;
        config.disable_mapping_for_testing();
        config.development.as_mut().unwrap().disable_mapping = disable_mapping;
        config.development.as_mut().unwrap().enable_mapping_errors = true;
        let mut schema = eql_mapper::Schema::new("public");
        let mut table = eql_mapper::Table::new(sqltk::parser::ast::Ident::new("encrypted"));
        table.add_column(Arc::new(eql_mapper::Column::eql(
            sqltk::parser::ast::Ident::new("secret"),
            eql_mapper::EqlTraits::none(),
            eql_mapper::DomainIdentity::from_domain_name("eql_v3_text").unwrap(),
        )));
        schema.add_table(table);
        let mut encryption = EncryptConfig::new();
        encryption.insert(
            crate::Identifier::new("encrypted", "secret"),
            cipherstash_client::schema::ColumnConfig::build("secret".to_owned())
                .casts_as(cipherstash_client::schema::ColumnType::Text),
        );
        let cipher = Cipher::default();
        let (sender, _) = tokio::sync::mpsc::unbounded_channel();
        let context = Context::new(
            1,
            Arc::new(config),
            Arc::new(encryption),
            Arc::new(schema),
            Arc::new(rustls::RootCertStore::empty()),
            cipher.clone(),
            sender,
        );
        Self {
            frontend: Frontend::new(context.clone()),
            backend: Backend::new(context.clone()),
            context,
            cipher,
        }
    }

    async fn command(&mut self, sql: &str, tag: &str, status: TransactionStatus) {
        let operation = operation_id();
        self.frontend
            .intercept(
                operation,
                FrontendMessage::Query(Bytes::copy_from_slice(sql.as_bytes())),
            )
            .await
            .unwrap();
        self.backend
            .intercept(
                Some(operation),
                BackendMessage::CommandComplete(Bytes::copy_from_slice(tag.as_bytes())),
            )
            .await
            .unwrap();
        self.backend
            .intercept(Some(operation), BackendMessage::ReadyForQuery(status))
            .await
            .unwrap();
    }

    async fn encrypted_insert_uses(&mut self, expected: Option<KeysetIdentifier>) {
        self.cipher.selected.lock().unwrap().clear();
        self.frontend
            .intercept(
                operation_id(),
                FrontendMessage::Query(Bytes::from_static(
                    b"INSERT INTO encrypted(secret) VALUES ('value')",
                )),
            )
            .await
            .unwrap();
        assert_eq!(*self.cipher.selected.lock().unwrap(), vec![expected]);
    }

    async fn prepare(&mut self, sql: &str) {
        let operation = operation_id();
        self.frontend
            .intercept(
                operation,
                FrontendMessage::Parse(pg_proto::Parse {
                    statement: Bytes::from_static(b"setting"),
                    query: Bytes::copy_from_slice(sql.as_bytes()),
                    parameter_types: Vec::new(),
                }),
            )
            .await
            .unwrap();
        self.backend
            .intercept(Some(operation), BackendMessage::ParseComplete)
            .await
            .unwrap();
    }

    async fn execute_prepared(&mut self) {
        let operation = operation_id();
        self.frontend
            .intercept(
                operation,
                FrontendMessage::Bind(pg_proto::Bind {
                    portal: Bytes::new(),
                    statement: Bytes::from_static(b"setting"),
                    parameter_formats: Vec::new(),
                    parameters: Vec::new(),
                    result_formats: Vec::new(),
                }),
            )
            .await
            .unwrap();
        self.backend
            .intercept(Some(operation), BackendMessage::BindComplete)
            .await
            .unwrap();
        let operation = operation_id();
        self.frontend
            .intercept(
                operation,
                FrontendMessage::Execute(pg_proto::Execute {
                    portal: Bytes::new(),
                    max_rows: 0,
                }),
            )
            .await
            .unwrap();
        self.backend
            .intercept(
                Some(operation),
                BackendMessage::CommandComplete(Bytes::from_static(b"SET")),
            )
            .await
            .unwrap();
    }

    async fn sync(&mut self, status: TransactionStatus) {
        let operation = operation_id();
        self.frontend
            .intercept(operation, FrontendMessage::Sync)
            .await
            .unwrap();
        self.backend
            .intercept(Some(operation), BackendMessage::ReadyForQuery(status))
            .await
            .unwrap();
    }

    async fn fail_query(&mut self, status: TransactionStatus) {
        let operation = operation_id();
        self.frontend
            .intercept(
                operation,
                FrontendMessage::Query(Bytes::from_static(b"SELECT 1 / 0")),
            )
            .await
            .unwrap();
        self.backend
            .intercept(
                Some(operation),
                BackendMessage::ErrorResponse(
                    crate::postgresql::diagnostics::invalid_sql_statement(
                        "division by zero".to_owned(),
                    ),
                ),
            )
            .await
            .unwrap();
        // The Query boundary, not ReadyForQuery, ends the discard state.
        assert!(!self.context.discarding_extended_messages().unwrap());
        self.backend
            .intercept(Some(operation), BackendMessage::ReadyForQuery(status))
            .await
            .unwrap();
    }

    async fn query_is_forwarded(&mut self) {
        assert!(!self.context.discarding_extended_messages().unwrap());
        let output = self
            .frontend
            .intercept(
                operation_id(),
                FrontendMessage::Query(Bytes::from_static(b"SELECT 1")),
            )
            .await
            .unwrap();
        assert!(
            matches!(
                output,
                FrontendMiddlewareOutput::Forward(FrontendMessage::Query(_))
            ),
            "{output:?}"
        );
    }
}

fn named(name: &str) -> Option<KeysetIdentifier> {
    Some(KeysetIdentifier(IdentifiedBy::Name(name.to_owned().into())))
}

#[tokio::test]
async fn rollback_to_savepoint_restores_keyset_used_for_encryption() {
    let mut client = Client::new();
    client
        .command("BEGIN", "BEGIN", TransactionStatus::InTransaction)
        .await;
    client
        .command(
            "SET LOCAL CIPHERSTASH.KEYSET_NAME = 'original'",
            "SET",
            TransactionStatus::InTransaction,
        )
        .await;
    client
        .command(
            "SAVEPOINT before_change",
            "SAVEPOINT",
            TransactionStatus::InTransaction,
        )
        .await;
    client
        .command(
            "SET CIPHERSTASH.KEYSET_NAME = 'other'",
            "SET",
            TransactionStatus::InTransaction,
        )
        .await;
    client
        .command(
            "ROLLBACK TO before_change",
            "ROLLBACK",
            TransactionStatus::InTransaction,
        )
        .await;
    client.encrypted_insert_uses(named("original")).await;
}

#[tokio::test]
async fn local_prepared_outside_transaction_has_no_effect_until_executed_inside_begin() {
    let mut outside = Client::new();
    outside
        .prepare("SET LOCAL CIPHERSTASH.KEYSET_NAME = 'local'")
        .await;
    outside.sync(TransactionStatus::Idle).await;
    outside.execute_prepared().await;
    // Sync ends this implicit transaction; no setting leaks to the next query.
    outside.sync(TransactionStatus::Idle).await;
    outside.encrypted_insert_uses(None).await;

    let mut inside = Client::new();
    inside
        .prepare("SET LOCAL CIPHERSTASH.KEYSET_NAME = 'local'")
        .await;
    inside.sync(TransactionStatus::Idle).await;
    inside
        .command("BEGIN", "BEGIN", TransactionStatus::InTransaction)
        .await;
    inside.execute_prepared().await;
    inside.sync(TransactionStatus::InTransaction).await;
    inside.encrypted_insert_uses(named("local")).await;
}

#[tokio::test]
async fn preparing_keyset_does_not_change_encryption_and_each_execution_reapplies_it() {
    for execute in [false, true] {
        let mut client = Client::new();
        client
            .command(
                "SET CIPHERSTASH.KEYSET_NAME = 'original'",
                "SET",
                TransactionStatus::Idle,
            )
            .await;
        client
            .prepare("SET CIPHERSTASH.KEYSET_NAME = 'prepared'")
            .await;
        client.sync(TransactionStatus::Idle).await;
        if execute {
            client.execute_prepared().await;
            client.sync(TransactionStatus::Idle).await;
            client
                .command(
                    "SET CIPHERSTASH.KEYSET_NAME = 'other'",
                    "SET",
                    TransactionStatus::Idle,
                )
                .await;
            client.execute_prepared().await;
            client.sync(TransactionStatus::Idle).await;
        }
        client
            .encrypted_insert_uses(named(if execute { "prepared" } else { "original" }))
            .await;
    }
}

#[tokio::test]
async fn rollback_restores_keyset_id_and_mapping_before_next_encrypted_insert() {
    let mut client = Client::new();
    let id = "7d4cbd7f-ba0d-4985-9ed2-ebe2ffe77590";
    client
        .command(
            &format!("SET CIPHERSTASH.KEYSET_ID = '{id}'"),
            "SET",
            TransactionStatus::Idle,
        )
        .await;
    client
        .command("BEGIN", "BEGIN", TransactionStatus::InTransaction)
        .await;
    client
        .command(
            "SET CIPHERSTASH.KEYSET_NAME = 'other'",
            "SET",
            TransactionStatus::InTransaction,
        )
        .await;
    client
        .command(
            "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
            "SET",
            TransactionStatus::InTransaction,
        )
        .await;
    client
        .command("ROLLBACK", "ROLLBACK", TransactionStatus::Idle)
        .await;
    client
        .encrypted_insert_uses(Some(KeysetIdentifier(IdentifiedBy::Uuid(
            uuid::Uuid::parse_str(id).unwrap(),
        ))))
        .await;
}

#[tokio::test]
async fn released_savepoint_changes_remain_subject_to_outer_rollback() {
    let mut client = Client::new();
    client
        .command(
            "SET CIPHERSTASH.KEYSET_NAME = 'original'",
            "SET",
            TransactionStatus::Idle,
        )
        .await;
    client
        .command("BEGIN", "BEGIN", TransactionStatus::InTransaction)
        .await;
    client
        .command(
            "SAVEPOINT outer_point",
            "SAVEPOINT",
            TransactionStatus::InTransaction,
        )
        .await;
    client
        .command(
            "SAVEPOINT inner_point",
            "SAVEPOINT",
            TransactionStatus::InTransaction,
        )
        .await;
    client
        .command(
            "SET CIPHERSTASH.KEYSET_NAME = 'other'",
            "SET",
            TransactionStatus::InTransaction,
        )
        .await;
    client
        .command(
            "RELEASE outer_point",
            "RELEASE",
            TransactionStatus::InTransaction,
        )
        .await;
    client
        .command("ROLLBACK", "ROLLBACK", TransactionStatus::Idle)
        .await;
    client.encrypted_insert_uses(named("original")).await;
}

#[tokio::test]
async fn failed_transaction_commit_restores_encryption_keyset_and_mapping() {
    let mut client = Client::new();
    client
        .command(
            "SET CIPHERSTASH.KEYSET_NAME = 'original'",
            "SET",
            TransactionStatus::Idle,
        )
        .await;
    client
        .command("BEGIN", "BEGIN", TransactionStatus::InTransaction)
        .await;
    client
        .command(
            "SET CIPHERSTASH.KEYSET_NAME = 'other'",
            "SET",
            TransactionStatus::InTransaction,
        )
        .await;
    client
        .command(
            "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
            "SET",
            TransactionStatus::InTransaction,
        )
        .await;
    client
        .fail_query(TransactionStatus::FailedTransaction)
        .await;
    client
        .command("COMMIT", "ROLLBACK", TransactionStatus::Idle)
        .await;
    client.encrypted_insert_uses(named("original")).await;
}

#[tokio::test]
async fn rollback_to_savepoint_recovers_encryption_after_backend_error() {
    let mut client = Client::new();
    client
        .command("BEGIN", "BEGIN", TransactionStatus::InTransaction)
        .await;
    client
        .command(
            "SET LOCAL CIPHERSTASH.KEYSET_NAME = 'original'",
            "SET",
            TransactionStatus::InTransaction,
        )
        .await;
    client
        .command(
            "SAVEPOINT recovery",
            "SAVEPOINT",
            TransactionStatus::InTransaction,
        )
        .await;
    client
        .command(
            "SET LOCAL CIPHERSTASH.KEYSET_NAME = 'other'",
            "SET",
            TransactionStatus::InTransaction,
        )
        .await;
    client
        .command(
            "SET LOCAL CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
            "SET",
            TransactionStatus::InTransaction,
        )
        .await;
    client
        .fail_query(TransactionStatus::FailedTransaction)
        .await;
    client
        .command(
            "ROLLBACK TO recovery",
            "ROLLBACK",
            TransactionStatus::InTransaction,
        )
        .await;
    client.encrypted_insert_uses(named("original")).await;
}

#[tokio::test]
async fn transaction_completion_restores_session_keyset_and_enables_mapping() {
    for completion in ["COMMIT", "ROLLBACK"] {
        let mut client = Client::new();
        client
            .command(
                "SET CIPHERSTASH.KEYSET_NAME = 'session'",
                "SET",
                TransactionStatus::Idle,
            )
            .await;
        client
            .command("BEGIN", "BEGIN", TransactionStatus::InTransaction)
            .await;
        client
            .command(
                "SET LOCAL CIPHERSTASH.KEYSET_NAME = 'local'",
                "SET",
                TransactionStatus::InTransaction,
            )
            .await;
        client
            .command(
                "SET LOCAL CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
                "SET",
                TransactionStatus::InTransaction,
            )
            .await;
        client
            .command(completion, completion, TransactionStatus::Idle)
            .await;
        client.encrypted_insert_uses(named("session")).await;
    }
}

#[tokio::test]
async fn failed_simple_query_with_mapping_disabled_does_not_suppress_next_query() {
    for status in [
        TransactionStatus::Idle,
        TransactionStatus::FailedTransaction,
    ] {
        let mut client = Client::with_mapping_disabled(true);
        client.fail_query(status).await;
        client.query_is_forwarded().await;
    }
}

#[tokio::test]
async fn failed_pipelined_execute_with_mapping_disabled_does_not_suppress_next_query() {
    let mut client = Client::with_mapping_disabled(true);
    let (parse, bind, execute, sync) = (
        operation_id(),
        operation_id(),
        operation_id(),
        operation_id(),
    );
    // The driver sends the whole batch before PostgreSQL answers any of it.
    for (operation, message) in [
        (
            parse,
            FrontendMessage::Parse(pg_proto::Parse {
                statement: Bytes::new(),
                query: Bytes::from_static(b"INSERT INTO plain VALUES (1)"),
                parameter_types: Vec::new(),
            }),
        ),
        (
            bind,
            FrontendMessage::Bind(pg_proto::Bind {
                portal: Bytes::new(),
                statement: Bytes::new(),
                parameter_formats: Vec::new(),
                parameters: Vec::new(),
                result_formats: Vec::new(),
            }),
        ),
        (
            execute,
            FrontendMessage::Execute(pg_proto::Execute {
                portal: Bytes::new(),
                max_rows: 0,
            }),
        ),
        (sync, FrontendMessage::Sync),
    ] {
        client.frontend.intercept(operation, message).await.unwrap();
    }
    for (operation, message) in [
        (parse, BackendMessage::ParseComplete),
        (bind, BackendMessage::BindComplete),
        (
            execute,
            BackendMessage::ErrorResponse(crate::postgresql::diagnostics::invalid_sql_statement(
                "duplicate key value".to_owned(),
            )),
        ),
    ] {
        client
            .backend
            .intercept(Some(operation), message)
            .await
            .unwrap();
    }

    // The driver pipelined this Query, so it can arrive before ReadyForQuery.
    client.query_is_forwarded().await;
    client
        .backend
        .intercept(
            Some(sync),
            BackendMessage::ReadyForQuery(TransactionStatus::Idle),
        )
        .await
        .unwrap();
}
