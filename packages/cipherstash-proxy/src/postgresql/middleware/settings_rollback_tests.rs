//! Behavioral transaction-setting tests at the PostgreSQL message boundary.

use super::{Backend, Frontend};
use crate::postgresql::{test_operation_id as operation_id, OperationId};
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
    decrypt_fails: Arc<std::sync::atomic::AtomicBool>,
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
        if self
            .decrypt_fails
            .load(std::sync::atomic::Ordering::Relaxed)
        {
            return Err(EncryptError::InvalidInboundEqlPayload.into());
        }
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
    cipher: Cipher,
    context: Context<Cipher>,
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
            cipher,
            context,
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

    /// Runs `sql` as a simple Query, or through Parse, Bind, Execute and Sync of the
    /// unnamed statement
    async fn run(&mut self, extended: bool, sql: &str, tag: &str, status: TransactionStatus) {
        if extended {
            self.pipeline(&[(sql, Some(tag))], status).await;
        } else {
            self.command(sql, tag, status).await;
        }
    }

    /// Sends `statements` as one extended-protocol batch, each through Parse, Bind and
    /// Execute of the unnamed statement, followed by one Sync. A `None` tag is an
    /// Execute PostgreSQL rejects, after which it skips the rest of the batch.
    ///
    /// Like the driver, this reads PostgreSQL's replies only when an Execute leaves a
    /// settings change in flight, and otherwise keeps sending the batch.
    async fn pipeline(&mut self, statements: &[(&str, Option<&str>)], status: TransactionStatus) {
        let mut replies = Vec::new();
        let mut failed = false;
        for (sql, tag) in statements {
            let parse = operation_id();
            self.frontend
                .intercept(
                    parse,
                    FrontendMessage::Parse(pg_proto::Parse {
                        statement: Bytes::new(),
                        query: Bytes::copy_from_slice(sql.as_bytes()),
                        parameter_types: Vec::new(),
                    }),
                )
                .await
                .unwrap();
            let bind = operation_id();
            self.frontend
                .intercept(
                    bind,
                    FrontendMessage::Bind(pg_proto::Bind {
                        portal: Bytes::new(),
                        statement: Bytes::new(),
                        parameter_formats: Vec::new(),
                        parameters: Vec::new(),
                        result_formats: Vec::new(),
                    }),
                )
                .await
                .unwrap();
            let execute = operation_id();
            self.frontend
                .intercept(
                    execute,
                    FrontendMessage::Execute(pg_proto::Execute {
                        portal: Bytes::new(),
                        max_rows: 0,
                    }),
                )
                .await
                .unwrap();
            if !failed {
                replies.push((parse, BackendMessage::ParseComplete));
                replies.push((bind, BackendMessage::BindComplete));
                replies.push((
                    execute,
                    match tag {
                        Some(tag) => {
                            BackendMessage::CommandComplete(Bytes::copy_from_slice(tag.as_bytes()))
                        }
                        None => BackendMessage::ErrorResponse(
                            crate::postgresql::diagnostics::invalid_sql_statement(
                                "rejected".to_owned(),
                            ),
                        ),
                    },
                ));
                failed = tag.is_none();
            }
            if self.context.settings_in_flight().unwrap() {
                self.reply(&mut replies).await;
            }
        }
        let sync = operation_id();
        self.frontend
            .intercept(sync, FrontendMessage::Sync)
            .await
            .unwrap();
        replies.push((sync, BackendMessage::ReadyForQuery(status)));
        self.reply(&mut replies).await;
    }

    /// Delivers PostgreSQL's queued replies to the backend, in order
    async fn reply(&mut self, replies: &mut Vec<(OperationId, BackendMessage)>) {
        for (operation, message) in replies.drain(..) {
            self.backend
                .intercept(Some(operation), message)
                .await
                .unwrap();
        }
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

/// A decryption failure on the first statement of a simple query must not shift the
/// settings outcomes of the statements PostgreSQL ran after it.
#[tokio::test]
async fn decryption_failure_keeps_later_settings_in_step_with_postgresql() {
    let mut client = Client::new();
    client
        .cipher
        .decrypt_fails
        .store(true, std::sync::atomic::Ordering::Relaxed);
    let query = operation_id();
    client
        .frontend
        .intercept(
            query,
            FrontendMessage::Query(Bytes::from_static(
                b"SELECT secret FROM encrypted; SET CIPHERSTASH.KEYSET_NAME = 'a'; SET CIPHERSTASH.KEYSET_NAME = 'b'",
            )),
        )
        .await
        .unwrap();
    let field = pg_proto::FieldDescription {
        name: Bytes::from_static(b"secret"),
        table_oid: 0,
        column: 0,
        type_oid: 3802,
        type_size: -1,
        type_modifier: -1,
        format: 0,
    };
    for message in [
        BackendMessage::RowDescription(pg_proto::RowDescription {
            fields: vec![field],
        }),
        BackendMessage::DataRow(pg_proto::DataRow {
            columns: vec![Some(Bytes::from_static(b"not a ciphertext"))],
        }),
        BackendMessage::CommandComplete(Bytes::from_static(b"SELECT 1")),
        BackendMessage::CommandComplete(Bytes::from_static(b"SET")),
        BackendMessage::CommandComplete(Bytes::from_static(b"SET")),
        BackendMessage::ReadyForQuery(TransactionStatus::Idle),
    ] {
        client
            .backend
            .intercept(Some(query), message)
            .await
            .unwrap();
    }

    // PostgreSQL ran both SETs, so the keyset is the last one
    assert_eq!(client.context.keyset_identifier().unwrap(), named("b"));
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

/// Rolls back, either the whole transaction or to a savepoint taken after BEGIN, every
/// change in `changes` made over `original` settings, and checks the next encrypted
/// INSERT uses the keyset selected before the transaction.
async fn rollback_restores_original_keyset(
    extended: bool,
    to_savepoint: bool,
    changes: &[(&str, &str)],
) {
    use TransactionStatus::{Idle, InTransaction};
    let mut client = Client::new();
    client
        .run(
            extended,
            "SET CIPHERSTASH.KEYSET_NAME = 'original'",
            "SET",
            Idle,
        )
        .await;
    client.run(extended, "BEGIN", "BEGIN", InTransaction).await;
    if to_savepoint {
        client
            .run(
                extended,
                "SAVEPOINT before_change",
                "SAVEPOINT",
                InTransaction,
            )
            .await;
    }
    for (sql, tag) in changes {
        client.run(extended, sql, tag, InTransaction).await;
    }
    assert_ne!(
        (
            client.context.keyset_identifier().unwrap(),
            client.context.unsafe_disable_mapping().unwrap()
        ),
        (named("original"), false),
        "{changes:?} changed nothing"
    );
    if to_savepoint {
        client
            .run(
                extended,
                "ROLLBACK TO before_change",
                "ROLLBACK",
                InTransaction,
            )
            .await;
    } else {
        client.run(extended, "ROLLBACK", "ROLLBACK", Idle).await;
    }
    client.encrypted_insert_uses(named("original")).await;
}

#[tokio::test]
async fn rolled_back_keyset_name_reset_restores_the_selected_keyset() {
    for extended in [false, true] {
        for to_savepoint in [false, true] {
            for reset in [
                ("RESET CIPHERSTASH.KEYSET_NAME", "RESET"),
                ("SET CIPHERSTASH.KEYSET_NAME TO DEFAULT", "SET"),
            ] {
                rollback_restores_original_keyset(extended, to_savepoint, &[reset]).await;
            }
        }
    }
}

#[tokio::test]
async fn rolled_back_extended_protocol_set_and_reset_restore_settings() {
    for to_savepoint in [false, true] {
        for changes in [
            &[("SET CIPHERSTASH.KEYSET_NAME = 'other'", "SET")][..],
            &[("SET LOCAL CIPHERSTASH.KEYSET_NAME = 'other'", "SET")],
            &[("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET")],
            &[("RESET CIPHERSTASH.KEYSET_ID", "RESET")],
            &[("RESET ALL", "RESET")],
            &[
                ("SET CIPHERSTASH.KEYSET_NAME = 'other'", "SET"),
                ("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET"),
                ("RESET CIPHERSTASH.KEYSET_NAME", "RESET"),
            ],
        ] {
            rollback_restores_original_keyset(true, to_savepoint, changes).await;
        }
    }
}

#[tokio::test]
async fn pipelined_batch_that_changes_settings_and_rolls_back_restores_them() {
    use TransactionStatus::{Idle, InTransaction};
    let changes = [
        ("SET CIPHERSTASH.KEYSET_NAME = 'other'", Some("SET")),
        ("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", Some("SET")),
    ];
    let full_rollback = [
        &[("BEGIN", Some("BEGIN"))][..],
        &changes,
        &[("ROLLBACK", Some("ROLLBACK"))],
    ]
    .concat();
    let savepoint_rollback = [
        &[
            ("BEGIN", Some("BEGIN")),
            ("SAVEPOINT before_change", Some("SAVEPOINT")),
        ][..],
        &changes,
        &[("ROLLBACK TO before_change", Some("ROLLBACK"))],
    ]
    .concat();
    // Without a rollback, the batch's changes take effect
    let mut client = Client::new();
    client.pipeline(&changes, Idle).await;
    assert_eq!(client.context.keyset_identifier().unwrap(), named("other"));
    assert!(client.context.unsafe_disable_mapping().unwrap());

    // The batch's implicit transaction is rolled back when a later statement fails
    let implicit_abort = [&changes[..], &[("SELECT 1 / 0", None)]].concat();
    for (batch, status) in [
        (full_rollback, Idle),
        (savepoint_rollback, InTransaction),
        (implicit_abort, Idle),
    ] {
        let mut client = Client::new();
        client
            .command("SET CIPHERSTASH.KEYSET_NAME = 'original'", "SET", Idle)
            .await;
        client.pipeline(&batch, status).await;
        client.encrypted_insert_uses(named("original")).await;
    }
}

/// On PostgreSQL 14 to 17, `DISCARD ALL` is accepted only as the first Execute of an
/// extended-protocol batch, and is committed at once. A later failure in the batch
/// rolls back only what follows it.
#[tokio::test]
async fn discard_all_that_opens_a_pipeline_is_kept_when_a_later_statement_fails() {
    let mut client = Client::new();
    client
        .command(
            "SET CIPHERSTASH.KEYSET_NAME = 'original'",
            "SET",
            TransactionStatus::Idle,
        )
        .await;
    client
        .pipeline(
            &[
                ("DISCARD ALL", Some("DISCARD ALL")),
                ("SET CIPHERSTASH.KEYSET_NAME = 'after'", Some("SET")),
                ("SELECT 1 / 0", None),
            ],
            TransactionStatus::Idle,
        )
        .await;
    client.encrypted_insert_uses(None).await;
}

/// PostgreSQL rejects `DISCARD ALL` after the first Execute of a batch with
/// "DISCARD ALL cannot be executed within a pipeline"
#[tokio::test]
async fn discard_all_rejected_later_in_a_pipeline_keeps_settings() {
    let mut client = Client::new();
    client
        .command(
            "SET CIPHERSTASH.KEYSET_NAME = 'original'",
            "SET",
            TransactionStatus::Idle,
        )
        .await;
    client
        .pipeline(
            &[("SELECT 1", Some("SELECT 1")), ("DISCARD ALL", None)],
            TransactionStatus::Idle,
        )
        .await;
    client.encrypted_insert_uses(named("original")).await;
}
