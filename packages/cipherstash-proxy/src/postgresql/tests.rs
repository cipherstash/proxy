//! Tests that drive the frontend and backend of one connection together with the messages a
//! client and PostgreSQL exchange.

use super::{backend::Backend, frontend::Frontend, Column, Context, KeysetIdentifier};
use crate::{
    config::TandemConfig,
    error::Error,
    proxy::{EncryptConfig, EncryptionService},
};
use bytes::{BufMut, BytesMut};
use eql_mapper::Schema;
use std::sync::Arc;
use tokio::{
    io::{AsyncWriteExt, DuplexStream, ReadHalf, Sink},
    sync::mpsc,
};

#[derive(Clone)]
struct TestService;

#[async_trait::async_trait]
impl EncryptionService for TestService {
    async fn encrypt(
        &self,
        _keyset_id: Option<KeysetIdentifier>,
        _plaintexts: Vec<Option<cipherstash_client::encryption::Plaintext>>,
        _columns: &[Option<Column>],
    ) -> Result<Vec<Option<crate::EqlCiphertext>>, Error> {
        Ok(vec![])
    }

    async fn decrypt(
        &self,
        _keyset_id: Option<KeysetIdentifier>,
        _ciphertexts: Vec<Option<crate::EqlCiphertext>>,
    ) -> Result<Vec<Option<cipherstash_client::encryption::Plaintext>>, Error> {
        Ok(vec![])
    }
}

/// One client connection through Proxy.
///
/// [`Connection::client`] sends a message from the client through the frontend, and
/// [`Connection::server`] sends a message from PostgreSQL through the backend.
struct Connection {
    client: DuplexStream,
    frontend: Frontend<ReadHalf<DuplexStream>, Sink, TestService>,
    server: DuplexStream,
    backend: Backend<ReadHalf<DuplexStream>, TestService>,
    to_client: mpsc::UnboundedReceiver<BytesMut>,
    context: Context<TestService>,
}

impl Connection {
    fn new() -> Self {
        Self::with_config(TandemConfig::for_testing())
    }

    fn with_config(config: TandemConfig) -> Self {
        let (reload_sender, _) = mpsc::unbounded_channel();
        let context = Context::new(
            1,
            Arc::new(config),
            Arc::new(EncryptConfig::default()),
            Arc::new(Schema::new("public")),
            TestService,
            reload_sender,
        );

        let (client, client_reader) = tokio::io::duplex(64 * 1024);
        let (client_reader, _) = tokio::io::split(client_reader);
        let (server, server_reader) = tokio::io::duplex(64 * 1024);
        let (server_reader, _) = tokio::io::split(server_reader);
        let (client_sender, to_client) = mpsc::unbounded_channel();

        Self {
            client,
            frontend: Frontend::new(
                client_reader,
                client_sender.clone(),
                tokio::io::sink(),
                context.clone(),
            ),
            server,
            backend: Backend::new(client_sender, server_reader, context.clone()),
            to_client,
            context,
        }
    }

    /// A connection that has finished starting up
    async fn started() -> Self {
        let mut connection = Self::new();
        connection.server(ready(b'I')).await;
        connection
    }

    /// The client sends `message`
    async fn client(&mut self, message: BytesMut) {
        self.client.write_all(&message).await.unwrap();
        self.frontend.rewrite().await.unwrap();
    }

    /// PostgreSQL sends `message`
    async fn server(&mut self, message: BytesMut) {
        self.server.write_all(&message).await.unwrap();
        self.backend.rewrite().await.unwrap();
    }

    /// Send a simple Query and reply with PostgreSQL's `CommandComplete` and `ReadyForQuery`
    async fn simple(&mut self, sql: &str, tag: &str) {
        self.client(query(sql)).await;
        self.server(command_complete(tag)).await;
        self.server(ready(b'I')).await;
    }

    /// Parse, Bind, Execute and Sync `sql` as the unnamed statement, with PostgreSQL's
    /// responses
    async fn extended(&mut self, sql: &str, tag: &str) {
        self.client(parse("", sql)).await;
        self.client(bind("", "")).await;
        self.client(execute("")).await;
        self.client(sync()).await;
        self.server(parse_complete()).await;
        self.server(bind_complete()).await;
        self.server(command_complete(tag)).await;
        self.server(ready(b'I')).await;
    }

    /// The number of `ErrorResponse` messages sent to the client so far, whether from Proxy or
    /// PostgreSQL
    fn errors_sent_to_client(&mut self) -> usize {
        let mut errors = 0;
        while let Ok(message) = self.to_client.try_recv() {
            if message.first() == Some(&b'E') {
                errors += 1;
            }
        }
        errors
    }
}

fn message(code: u8, body: &[u8]) -> BytesMut {
    let mut bytes = BytesMut::new();
    bytes.put_u8(code);
    bytes.put_i32(body.len() as i32 + 4);
    bytes.put_slice(body);
    bytes
}

fn cstring(bytes: &mut Vec<u8>, value: &str) {
    bytes.extend_from_slice(value.as_bytes());
    bytes.push(0);
}

fn query(sql: &str) -> BytesMut {
    let mut body = vec![];
    cstring(&mut body, sql);
    message(b'Q', &body)
}

fn parse(statement: &str, sql: &str) -> BytesMut {
    let mut body = vec![];
    cstring(&mut body, statement);
    cstring(&mut body, sql);
    // No parameter types
    body.extend_from_slice(&0i16.to_be_bytes());
    message(b'P', &body)
}

fn bind(portal: &str, statement: &str) -> BytesMut {
    let mut body = vec![];
    cstring(&mut body, portal);
    cstring(&mut body, statement);
    // No parameter formats, parameters or result formats
    body.extend_from_slice(&0i16.to_be_bytes());
    body.extend_from_slice(&0i16.to_be_bytes());
    body.extend_from_slice(&0i16.to_be_bytes());
    message(b'B', &body)
}

fn execute(portal: &str) -> BytesMut {
    let mut body = vec![];
    cstring(&mut body, portal);
    body.extend_from_slice(&0i32.to_be_bytes());
    message(b'E', &body)
}

fn close_statement(statement: &str) -> BytesMut {
    let mut body = vec![b'S'];
    cstring(&mut body, statement);
    message(b'C', &body)
}

fn sync() -> BytesMut {
    message(b'S', b"")
}

fn parse_complete() -> BytesMut {
    message(b'1', b"")
}

fn bind_complete() -> BytesMut {
    message(b'2', b"")
}

fn close_complete() -> BytesMut {
    message(b'3', b"")
}

fn command_complete(tag: &str) -> BytesMut {
    let mut body = vec![];
    cstring(&mut body, tag);
    message(b'C', &body)
}

fn ready(transaction_status: u8) -> BytesMut {
    message(b'Z', &[transaction_status])
}

fn error() -> BytesMut {
    message(b'E', b"SERROR\0C25P02\0Mcurrent transaction is aborted\0\0")
}

/// `DISCARD ALL` and `RESET` return the connection's Proxy settings to their startup values
/// once PostgreSQL confirms them, so a pooled connection does not carry one client's keyset or
/// disabled mapping over to the next client.
mod settings_reset {
    use super::*;

    const KEYSET_ID: &str = "SET CIPHERSTASH.KEYSET_ID = '7e5ea5a6-4e1c-4f4d-8a9c-9f0b4f6a0c11'";

    /// A started connection to a Proxy with no default keyset, so a keyset can be set per
    /// connection
    async fn connection() -> Connection {
        let mut config = TandemConfig::for_testing();
        config.encrypt.default_keyset_id = None;
        let mut connection = Connection::with_config(config);
        connection.server(ready(b'I')).await;
        connection
    }

    impl Connection {
        /// Select a keyset, then disable mapping
        async fn set_keyset_and_disable_mapping(&mut self) {
            self.simple(KEYSET_ID, "SET").await;
            self.simple("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET")
                .await;
            assert!(self.context.keyset_identifier().is_some());
            assert!(self.context.unsafe_disable_mapping());
        }

        /// Prepare `sql` as the named statement `name`
        async fn prepare(&mut self, name: &str, sql: &str) {
            self.client(parse(name, sql)).await;
            self.client(sync()).await;
            self.server(parse_complete()).await;
            self.server(ready(b'I')).await;
        }

        /// Bind and execute the named statement `name` through the unnamed portal
        async fn execute_prepared(&mut self, name: &str, tag: &str) {
            self.client(bind("", name)).await;
            self.client(execute("")).await;
            self.client(sync()).await;
            self.server(bind_complete()).await;
            self.server(command_complete(tag)).await;
            self.server(ready(b'I')).await;
        }

        fn assert_startup_settings(&mut self) {
            assert_eq!(self.context.keyset_identifier(), None);
            assert!(!self.context.unsafe_disable_mapping());
        }
    }

    #[tokio::test]
    async fn discard_all_in_a_simple_query() {
        let mut connection = connection().await;
        connection.set_keyset_and_disable_mapping().await;
        connection.simple("DISCARD ALL", "DISCARD ALL").await;
        connection.assert_startup_settings();
    }

    #[tokio::test]
    async fn reset_all_in_a_simple_query() {
        let mut connection = connection().await;
        connection.set_keyset_and_disable_mapping().await;
        connection.simple("RESET ALL", "RESET").await;
        // Forwarded to PostgreSQL, not rejected by Proxy
        assert_eq!(connection.errors_sent_to_client(), 0);
        connection.assert_startup_settings();
    }

    #[tokio::test]
    async fn discard_all_in_the_extended_protocol() {
        let mut connection = connection().await;
        connection.set_keyset_and_disable_mapping().await;
        connection.extended("DISCARD ALL", "DISCARD ALL").await;
        connection.assert_startup_settings();
    }

    #[tokio::test]
    async fn reset_all_in_the_extended_protocol() {
        let mut connection = connection().await;
        connection.set_keyset_and_disable_mapping().await;
        connection.extended("RESET ALL", "RESET").await;
        assert_eq!(connection.errors_sent_to_client(), 0);
        connection.assert_startup_settings();
    }

    #[tokio::test]
    async fn discard_all_falls_back_to_the_default_keyset() {
        let mut connection = Connection::started().await;
        connection
            .simple("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET")
            .await;
        connection.simple("DISCARD ALL", "DISCARD ALL").await;
        connection.assert_startup_settings();
        assert!(connection.context.default_keyset_id().is_some());
    }

    #[tokio::test]
    async fn reset_of_each_setting_resets_only_that_setting() {
        let mut connection = connection().await;
        connection.set_keyset_and_disable_mapping().await;

        // A PostgreSQL setting and the keyset name, which was never set
        connection.simple("RESET search_path", "RESET").await;
        connection
            .simple("RESET CIPHERSTASH.KEYSET_NAME", "RESET")
            .await;
        assert!(connection.context.keyset_identifier().is_some());
        assert!(connection.context.unsafe_disable_mapping());

        connection
            .simple("RESET CIPHERSTASH.UNSAFE_DISABLE_MAPPING", "RESET")
            .await;
        assert!(connection.context.keyset_identifier().is_some());
        assert!(!connection.context.unsafe_disable_mapping());

        connection
            .simple("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET")
            .await;
        connection
            .extended("RESET cipherstash.keyset_id", "RESET")
            .await;
        assert_eq!(connection.context.keyset_identifier(), None);
        assert!(connection.context.unsafe_disable_mapping());
    }

    #[tokio::test]
    async fn reset_of_the_keyset_name() {
        let mut connection = connection().await;
        connection
            .simple("SET CIPHERSTASH.KEYSET_NAME = 'tenant'", "SET")
            .await;
        connection
            .simple("RESET CIPHERSTASH.KEYSET_ID", "RESET")
            .await;
        assert!(connection.context.keyset_identifier().is_some());
        connection
            .simple("RESET CIPHERSTASH.KEYSET_NAME", "RESET")
            .await;
        assert_eq!(connection.context.keyset_identifier(), None);
    }

    #[tokio::test]
    async fn a_quoted_name_including_the_period_names_the_setting() {
        let mut connection = connection().await;
        connection.set_keyset_and_disable_mapping().await;

        connection
            .simple(r#"RESET "cipherstash.keyset_id""#, "RESET")
            .await;
        assert_eq!(connection.context.keyset_identifier(), None);

        connection
            .simple(
                r#"SET "CIPHERSTASH.UNSAFE_DISABLE_MAPPING" TO DEFAULT"#,
                "SET",
            )
            .await;
        assert!(!connection.context.unsafe_disable_mapping());
    }

    #[tokio::test]
    async fn set_to_default_resets_that_setting() {
        let mut connection = connection().await;
        connection.set_keyset_and_disable_mapping().await;

        connection
            .simple("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING TO DEFAULT", "SET")
            .await;
        assert!(connection.context.keyset_identifier().is_some());
        assert!(!connection.context.unsafe_disable_mapping());

        connection
            .extended("SET CIPHERSTASH.KEYSET_ID = default", "SET")
            .await;
        assert_eq!(connection.context.keyset_identifier(), None);
    }

    #[tokio::test]
    async fn set_keyset_name_to_default_is_not_a_keyset_named_default() {
        let mut connection = connection().await;
        connection
            .simple("SET CIPHERSTASH.KEYSET_NAME = 'tenant'", "SET")
            .await;
        connection
            .simple("SET CIPHERSTASH.KEYSET_NAME TO DEFAULT", "SET")
            .await;
        assert_eq!(connection.context.keyset_identifier(), None);

        // Not a request for a keyset, so not refused when there is a default keyset
        let mut connection = Connection::started().await;
        connection
            .simple("SET CIPHERSTASH.KEYSET_NAME TO DEFAULT", "SET")
            .await;
        connection
            .extended("SET CIPHERSTASH.KEYSET_NAME TO DEFAULT", "SET")
            .await;
        assert_eq!(connection.errors_sent_to_client(), 0);
    }

    #[tokio::test]
    async fn settings_are_kept_until_postgresql_confirms_the_reset() {
        let mut connection = connection().await;
        connection.set_keyset_and_disable_mapping().await;

        connection.client(query("DISCARD ALL")).await;
        assert!(connection.context.keyset_identifier().is_some());
        assert!(connection.context.unsafe_disable_mapping());

        connection.server(command_complete("DISCARD ALL")).await;
        connection.assert_startup_settings();
    }

    #[tokio::test]
    async fn a_failed_reset_keeps_the_settings() {
        let mut connection = connection().await;
        connection.set_keyset_and_disable_mapping().await;

        // Inside an aborted transaction PostgreSQL rejects each statement
        for sql in ["DISCARD ALL", "RESET ALL", "RESET ALL; RESET ALL"] {
            connection.client(query(sql)).await;
            connection.server(error()).await;
            connection.server(ready(b'E')).await;
        }

        connection.client(parse("", "RESET ALL")).await;
        connection.client(bind("", "")).await;
        connection.client(execute("")).await;
        connection.client(sync()).await;
        connection.server(parse_complete()).await;
        connection.server(bind_complete()).await;
        connection.server(error()).await;
        connection.server(ready(b'E')).await;

        // The next statement PostgreSQL confirms is not taken for the failed reset
        connection.simple("SET search_path = public", "SET").await;

        assert!(connection.context.keyset_identifier().is_some());
        assert!(connection.context.unsafe_disable_mapping());
        assert_eq!(connection.context.state_sizes().pending_settings_resets, 0);
    }

    #[tokio::test]
    async fn resets_in_one_query_apply_in_order() {
        let mut connection = connection().await;
        connection.set_keyset_and_disable_mapping().await;

        // The second RESET fails, so only the first is applied
        connection
            .client(query("RESET CIPHERSTASH.UNSAFE_DISABLE_MAPPING; RESET ALL"))
            .await;
        connection.server(command_complete("RESET")).await;
        connection.server(error()).await;
        connection.server(ready(b'I')).await;

        assert!(connection.context.keyset_identifier().is_some());
        assert!(!connection.context.unsafe_disable_mapping());
    }

    #[tokio::test]
    async fn set_to_default_after_other_sets_in_one_query() {
        let mut connection = connection().await;
        connection.set_keyset_and_disable_mapping().await;

        // Each SET is confirmed with its own tag, so only the second SET is a reset
        connection
            .client(query(
                "SET search_path = public; SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING TO DEFAULT; SET CIPHERSTASH.KEYSET_ID TO DEFAULT",
            ))
            .await;
        connection.server(command_complete("SET")).await;
        connection.server(command_complete("SET")).await;
        connection.server(error()).await;
        connection.server(ready(b'I')).await;

        assert!(connection.context.keyset_identifier().is_some());
        assert!(!connection.context.unsafe_disable_mapping());
    }

    #[tokio::test]
    async fn a_set_sent_after_a_reset_is_kept() {
        let mut connection = connection().await;
        connection.simple(KEYSET_ID, "SET").await;

        // The next client's SET is sent before PostgreSQL confirms the reset
        connection.client(query("DISCARD ALL")).await;
        connection.client(query(KEYSET_ID)).await;
        connection.server(command_complete("DISCARD ALL")).await;
        connection.server(ready(b'I')).await;
        connection.server(command_complete("SET")).await;
        connection.server(ready(b'I')).await;

        assert!(connection.context.keyset_identifier().is_some());
    }

    #[tokio::test]
    async fn a_set_of_one_setting_after_a_reset_keeps_the_other_reset() {
        let mut connection = connection().await;
        connection.set_keyset_and_disable_mapping().await;

        connection.client(parse("", "RESET ALL")).await;
        connection.client(bind("", "")).await;
        connection.client(execute("")).await;
        connection.client(sync()).await;
        connection
            .client(query("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true"))
            .await;
        connection.server(parse_complete()).await;
        connection.server(bind_complete()).await;
        connection.server(command_complete("RESET")).await;
        connection.server(ready(b'I')).await;
        connection.server(command_complete("SET")).await;
        connection.server(ready(b'I')).await;

        assert_eq!(connection.context.keyset_identifier(), None);
        assert!(connection.context.unsafe_disable_mapping());
    }

    #[tokio::test]
    async fn set_to_default_after_another_set_in_one_extended_batch() {
        let mut connection = connection().await;
        connection.set_keyset_and_disable_mapping().await;

        // Each Execute is confirmed with its own tag, so the reset waits for the second one,
        // which fails
        connection
            .client(parse("", "SET search_path = public"))
            .await;
        connection.client(bind("", "")).await;
        connection.client(execute("")).await;
        connection
            .client(parse("", "SET CIPHERSTASH.KEYSET_ID TO DEFAULT"))
            .await;
        connection.client(bind("", "")).await;
        connection.client(execute("")).await;
        connection.client(sync()).await;
        connection.server(parse_complete()).await;
        connection.server(bind_complete()).await;
        connection.server(command_complete("SET")).await;
        connection.server(parse_complete()).await;
        connection.server(bind_complete()).await;
        connection.server(error()).await;
        connection.server(ready(b'I')).await;

        assert!(connection.context.keyset_identifier().is_some());
    }

    #[tokio::test]
    async fn prepared_reset_runs_each_time_it_is_executed() {
        let mut connection = connection().await;
        connection.client(parse("reset", "RESET ALL")).await;
        connection.client(sync()).await;
        connection.server(parse_complete()).await;
        connection.server(ready(b'I')).await;

        for _ in 0..2 {
            connection.set_keyset_and_disable_mapping().await;
            connection.client(bind("", "reset")).await;
            connection.client(execute("")).await;
            connection.client(sync()).await;
            connection.server(bind_complete()).await;
            connection.server(command_complete("RESET")).await;
            connection.server(ready(b'I')).await;
            connection.assert_startup_settings();
        }
    }

    #[tokio::test]
    async fn closing_a_prepared_reset_retains_nothing() {
        let mut connection = connection().await;
        for i in 0..200 {
            let name = format!("reset_{i}");
            connection.client(parse(&name, "RESET ALL")).await;
            connection.client(bind("", &name)).await;
            connection.client(execute("")).await;
            connection.client(close_statement(&name)).await;
            connection.client(sync()).await;
            connection.server(parse_complete()).await;
            connection.server(bind_complete()).await;
            connection.server(command_complete("RESET")).await;
            connection.server(close_complete()).await;
            connection.server(ready(b'I')).await;
        }
        let sizes = connection.context.state_sizes();
        assert_eq!(sizes.statement_resets, 0);
        assert_eq!(sizes.portal_resets, 0);
        assert_eq!(sizes.pending_settings_resets, 0);
    }

    #[tokio::test]
    async fn a_prepared_keyset_set_applies_again_when_executed_after_a_reset() {
        let mut connection = connection().await;
        connection.prepare("set_keyset", KEYSET_ID).await;
        connection.execute_prepared("set_keyset", "SET").await;
        connection.simple("RESET ALL", "RESET").await;
        assert_eq!(connection.context.keyset_identifier(), None);

        connection.execute_prepared("set_keyset", "SET").await;
        assert!(connection.context.keyset_identifier().is_some());
    }

    #[tokio::test]
    async fn a_prepared_mapping_set_applies_again_when_executed_after_a_reset() {
        let mut connection = connection().await;
        connection
            .prepare("disable", "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true")
            .await;
        connection.execute_prepared("disable", "SET").await;
        connection.simple("DISCARD ALL", "DISCARD ALL").await;
        assert!(!connection.context.unsafe_disable_mapping());
        assert_eq!(connection.context.state_sizes().statement_settings, 0);

        connection
            .prepare("disable", "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true")
            .await;
        connection.simple("RESET ALL", "RESET").await;
        connection.execute_prepared("disable", "SET").await;
        assert!(connection.context.unsafe_disable_mapping());
    }

    #[tokio::test]
    async fn a_prepared_keyset_set_executed_after_a_pending_reset_is_kept() {
        let mut connection = connection().await;
        connection.simple(KEYSET_ID, "SET").await;
        connection.prepare("set_keyset", KEYSET_ID).await;

        // PostgreSQL runs the reset, then the prepared SET
        connection.client(query("RESET ALL")).await;
        connection.client(bind("", "set_keyset")).await;
        connection.client(execute("")).await;
        connection.client(sync()).await;
        connection.server(command_complete("RESET")).await;
        connection.server(ready(b'I')).await;
        connection.server(bind_complete()).await;
        connection.server(command_complete("SET")).await;
        connection.server(ready(b'I')).await;

        assert!(connection.context.keyset_identifier().is_some());
    }

    const OTHER_KEYSET_ID: &str =
        "SET CIPHERSTASH.KEYSET_ID = '0b9f3e2c-1d4a-4c6e-9f8a-2b7d5e1c3a90'";

    #[tokio::test]
    async fn a_reset_bound_before_the_previous_batch_is_ready_still_runs() {
        let mut connection = connection().await;
        connection.prepare("r", "RESET ALL").await;
        connection.set_keyset_and_disable_mapping().await;

        // The reset is bound before PostgreSQL answers the previous Query
        connection.client(query("SELECT 1")).await;
        connection.client(bind("", "r")).await;
        connection.server(command_complete("SELECT 1")).await;
        connection.server(ready(b'I')).await;
        connection.client(execute("")).await;
        connection.client(sync()).await;
        connection.server(bind_complete()).await;
        connection.server(command_complete("RESET")).await;
        connection.server(ready(b'I')).await;

        connection.assert_startup_settings();
    }

    #[tokio::test]
    async fn a_reset_bound_while_a_query_runs_still_runs_with_mapping_enabled() {
        let mut connection = connection().await;
        connection.prepare("r", "RESET ALL").await;
        connection.simple(KEYSET_ID, "SET").await;

        connection.client(query("SELECT 1")).await;
        connection.client(bind("", "r")).await;
        connection.server(command_complete("SELECT 1")).await;
        connection.server(ready(b'I')).await;
        connection.client(execute("")).await;
        connection.client(sync()).await;
        connection.server(bind_complete()).await;
        connection.server(command_complete("RESET")).await;
        connection.server(ready(b'I')).await;

        connection.assert_startup_settings();
    }

    #[tokio::test]
    async fn a_set_after_set_to_default_in_one_query_is_kept() {
        let mut connection = connection().await;
        connection.simple(KEYSET_ID, "SET").await;

        connection
            .client(query(&format!(
                "SET CIPHERSTASH.KEYSET_ID TO DEFAULT; {OTHER_KEYSET_ID}"
            )))
            .await;
        connection.server(command_complete("SET")).await;
        connection.server(command_complete("SET")).await;
        connection.server(ready(b'I')).await;

        assert!(connection.context.keyset_identifier().is_some());
    }

    #[tokio::test]
    async fn set_to_default_after_a_set_in_one_query_resets_it() {
        let mut connection = connection().await;

        connection
            .client(query(
                "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true; \
                 SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING TO DEFAULT",
            ))
            .await;
        connection.server(command_complete("SET")).await;
        connection.server(command_complete("SET")).await;
        connection.server(ready(b'I')).await;

        assert!(!connection.context.unsafe_disable_mapping());
    }

    #[tokio::test]
    async fn a_keyset_set_sent_before_mapping_is_turned_back_on_is_kept() {
        let mut connection = connection().await;
        connection.set_keyset_and_disable_mapping().await;
        let before = connection.context.keyset_identifier();

        connection
            .client(query("RESET CIPHERSTASH.UNSAFE_DISABLE_MAPPING"))
            .await;
        connection.client(query(OTHER_KEYSET_ID)).await;
        connection.server(command_complete("RESET")).await;
        connection.server(ready(b'I')).await;
        connection.server(command_complete("SET")).await;
        connection.server(ready(b'I')).await;

        assert!(!connection.context.unsafe_disable_mapping());
        assert!(connection.context.keyset_identifier().is_some());
        assert_ne!(connection.context.keyset_identifier(), before);
    }
}
