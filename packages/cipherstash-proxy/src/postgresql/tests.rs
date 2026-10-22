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
        pub(super) async fn prepare(&mut self, name: &str, sql: &str) {
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

        // A PostgreSQL setting
        connection.simple("RESET search_path", "RESET").await;
        assert!(connection.context.keyset_identifier().is_some());
        assert!(connection.context.unsafe_disable_mapping());
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
    async fn reset_of_either_keyset_setting_clears_the_keyset() {
        // Proxy keeps one keyset, so resetting either setting fails closed rather than keeping
        // a keyset chosen by the other one
        for reset in [
            "RESET CIPHERSTASH.KEYSET_ID",
            "RESET CIPHERSTASH.KEYSET_NAME",
        ] {
            let mut connection = connection().await;
            connection
                .simple("SET CIPHERSTASH.KEYSET_NAME = 'tenant'", "SET")
                .await;
            connection.simple(reset, "RESET").await;
            assert_eq!(connection.context.keyset_identifier(), None, "{reset}");
        }
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
        assert_eq!(connection.context.state_sizes().pending_settings, 0);
    }

    #[tokio::test]
    async fn a_failed_query_rolls_back_its_earlier_resets() {
        let mut connection = connection().await;
        connection.set_keyset_and_disable_mapping().await;

        // PostgreSQL runs the Query as one implicit transaction, so the second RESET's error
        // also rolls back the first
        connection
            .client(query("RESET CIPHERSTASH.UNSAFE_DISABLE_MAPPING; RESET ALL"))
            .await;
        connection.server(command_complete("RESET")).await;
        assert!(!connection.context.unsafe_disable_mapping());
        connection.server(error()).await;
        connection.server(ready(b'I')).await;

        assert!(connection.context.keyset_identifier().is_some());
        assert!(connection.context.unsafe_disable_mapping());
    }

    #[tokio::test]
    async fn resets_in_one_query_apply_in_order() {
        let mut connection = connection().await;
        connection.set_keyset_and_disable_mapping().await;

        connection
            .client(query("RESET CIPHERSTASH.UNSAFE_DISABLE_MAPPING; RESET ALL"))
            .await;
        connection.server(command_complete("RESET")).await;
        assert!(connection.context.keyset_identifier().is_some());
        assert!(!connection.context.unsafe_disable_mapping());
        connection.server(command_complete("RESET")).await;
        connection.server(ready(b'I')).await;

        connection.assert_startup_settings();
    }

    #[tokio::test]
    async fn a_failed_query_rolls_back_its_earlier_set_to_default() {
        let mut connection = connection().await;
        connection.set_keyset_and_disable_mapping().await;

        // The last SET fails, and PostgreSQL rolls back the implicit transaction, so the
        // earlier SET TO DEFAULT is undone with it
        connection
            .client(query(
                "SET search_path = public; SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING TO DEFAULT; SET CIPHERSTASH.KEYSET_ID TO DEFAULT",
            ))
            .await;
        connection.server(command_complete("SET")).await;
        connection.server(command_complete("SET")).await;
        // Each SET is confirmed with its own tag, so only the second SET is a reset
        assert!(connection.context.keyset_identifier().is_some());
        assert!(!connection.context.unsafe_disable_mapping());
        connection.server(error()).await;
        connection.server(ready(b'I')).await;

        assert!(connection.context.keyset_identifier().is_some());
        assert!(connection.context.unsafe_disable_mapping());
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
        assert_eq!(sizes.statement_settings, 0);
        assert_eq!(sizes.portal_settings, 0);
        assert_eq!(sizes.pending_settings, 0);
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

    #[tokio::test]
    async fn a_prepared_set_that_is_never_executed_changes_nothing() {
        let mut connection = connection().await;
        connection.prepare("set_keyset", KEYSET_ID).await;
        connection
            .prepare("disable", "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true")
            .await;
        connection.assert_startup_settings();
    }

    #[tokio::test]
    async fn a_prepared_keyset_set_that_is_never_executed_changes_nothing_while_mapping_is_disabled(
    ) {
        let mut connection = connection().await;
        connection
            .simple("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET")
            .await;
        connection.prepare("set_keyset", KEYSET_ID).await;
        assert_eq!(connection.context.keyset_identifier(), None);

        connection.execute_prepared("set_keyset", "SET").await;
        assert!(connection.context.keyset_identifier().is_some());
    }

    #[tokio::test]
    async fn sets_after_a_statement_that_fails_type_checking_still_apply() {
        let mut connection = connection().await;
        connection
            .client(query(&format!(
                "SELECT missing_column FROM missing_table; {KEYSET_ID}; \
                 SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true"
            )))
            .await;
        connection.server(command_complete("SELECT 0")).await;
        connection.server(command_complete("SET")).await;
        connection.server(command_complete("SET")).await;
        connection.server(ready(b'I')).await;

        assert!(connection.context.keyset_identifier().is_some());
        assert!(connection.context.unsafe_disable_mapping());
    }

    #[tokio::test]
    async fn an_invalid_keyset_id_is_refused_while_mapping_is_disabled() {
        let mut connection = connection().await;
        connection
            .simple("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET")
            .await;
        connection.errors_sent_to_client();
        connection
            .client(query("SET CIPHERSTASH.KEYSET_ID = 'not-a-uuid'"))
            .await;
        assert_eq!(connection.context.keyset_identifier(), None);
        assert_eq!(connection.errors_sent_to_client(), 1);
    }

    #[tokio::test]
    async fn a_keyset_set_with_a_default_keyset_is_refused_when_prepared() {
        let mut connection = Connection::started().await;
        connection.client(parse("set_keyset", KEYSET_ID)).await;
        assert_eq!(connection.errors_sent_to_client(), 1);
        assert_eq!(connection.context.keyset_identifier(), None);
    }

    #[tokio::test]
    async fn a_keyset_set_prepared_with_a_default_keyset_is_ignored_only_while_mapping_is_disabled()
    {
        let mut connection = Connection::started().await;
        connection
            .simple("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET")
            .await;
        connection.prepare("set_keyset", KEYSET_ID).await;
        connection.execute_prepared("set_keyset", "SET").await;
        assert_eq!(connection.errors_sent_to_client(), 0);
        assert_eq!(connection.context.keyset_identifier(), None);

        // Once mapping is enabled, PostgreSQL would accept the SET while Proxy kept encrypting
        // with the default keyset
        connection
            .simple("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = false", "SET")
            .await;
        connection.client(bind("", "set_keyset")).await;
        connection.client(execute("")).await;
        assert_eq!(connection.errors_sent_to_client(), 1);
        assert_eq!(connection.context.state_sizes().pending_settings, 0);
    }
}

/// `SET CIPHERSTASH.*` settings follow PostgreSQL's transaction rules: a change is kept on
/// `COMMIT`, undone on `ROLLBACK` or `ROLLBACK TO SAVEPOINT`, and `SET LOCAL` ends with the
/// transaction.
mod settings_transactions {
    use super::*;
    use cipherstash_client::IdentifiedBy;

    const KEYSET_A: &str = "7e5ea5a6-4e1c-4f4d-8a9c-9f0b4f6a0c11";
    const KEYSET_B: &str = "0b9f3e2c-1d4a-4c6e-9f8a-2b7d5e1c3a90";

    fn keyset(id: &str) -> Option<KeysetIdentifier> {
        Some(KeysetIdentifier(IdentifiedBy::Uuid(id.parse().unwrap())))
    }

    fn set_keyset(id: &str) -> String {
        format!("SET CIPHERSTASH.KEYSET_ID = '{id}'")
    }

    /// A started connection to a Proxy with no default keyset
    async fn connection() -> Connection {
        let mut config = TandemConfig::for_testing();
        config.encrypt.default_keyset_id = None;
        let mut connection = Connection::with_config(config);
        connection.server(ready(b'I')).await;
        connection
    }

    impl Connection {
        /// Send a simple Query and reply with `tag` and a `ReadyForQuery` with `status`
        async fn simple_in(&mut self, sql: &str, tag: &str, status: u8) {
            self.client(query(sql)).await;
            self.server(command_complete(tag)).await;
            self.server(ready(status)).await;
        }

        /// Parse, Bind, Execute and Sync `sql`, replying with `tag` and `status`
        async fn extended_in(&mut self, sql: &str, tag: &str, status: u8) {
            self.client(parse("", sql)).await;
            self.client(bind("", "")).await;
            self.client(execute("")).await;
            self.client(sync()).await;
            self.server(parse_complete()).await;
            self.server(bind_complete()).await;
            self.server(command_complete(tag)).await;
            self.server(ready(status)).await;
        }

        /// Send a simple Query that PostgreSQL rejects, leaving `status`
        async fn failed_in(&mut self, sql: &str, status: u8) {
            self.client(query(sql)).await;
            self.server(error()).await;
            self.server(ready(status)).await;
        }
    }

    #[tokio::test]
    async fn a_rolled_back_set_is_undone() {
        let mut connection = connection().await;
        connection.simple(&set_keyset(KEYSET_A), "SET").await;

        connection.simple_in("BEGIN", "BEGIN", b'T').await;
        connection
            .simple_in(&set_keyset(KEYSET_B), "SET", b'T')
            .await;
        connection
            .simple_in("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET", b'T')
            .await;
        assert_eq!(connection.context.keyset_identifier(), keyset(KEYSET_B));
        connection.simple_in("ROLLBACK", "ROLLBACK", b'I').await;

        assert_eq!(connection.context.keyset_identifier(), keyset(KEYSET_A));
        assert!(!connection.context.unsafe_disable_mapping());
    }

    #[tokio::test]
    async fn a_rolled_back_reset_is_undone() {
        for reset in [
            ("RESET ALL", "RESET"),
            ("RESET CIPHERSTASH.KEYSET_ID", "RESET"),
            ("SET CIPHERSTASH.KEYSET_ID TO DEFAULT", "SET"),
        ] {
            let mut connection = connection().await;
            connection.simple(&set_keyset(KEYSET_A), "SET").await;
            connection
                .simple("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET")
                .await;

            connection.simple_in("BEGIN", "BEGIN", b'T').await;
            connection.simple_in(reset.0, reset.1, b'T').await;
            assert_eq!(connection.context.keyset_identifier(), None, "{reset:?}");
            connection.simple_in("ROLLBACK", "ROLLBACK", b'I').await;

            assert_eq!(
                connection.context.keyset_identifier(),
                keyset(KEYSET_A),
                "{reset:?}"
            );
            assert!(connection.context.unsafe_disable_mapping(), "{reset:?}");
        }
    }

    #[tokio::test]
    async fn changes_in_a_committed_transaction_are_kept() {
        let mut connection = connection().await;
        connection.simple_in("BEGIN", "BEGIN", b'T').await;
        connection
            .simple_in(&set_keyset(KEYSET_A), "SET", b'T')
            .await;
        connection.simple_in("COMMIT", "COMMIT", b'I').await;
        assert_eq!(connection.context.keyset_identifier(), keyset(KEYSET_A));

        connection.simple_in("BEGIN", "BEGIN", b'T').await;
        connection.simple_in("RESET ALL", "RESET", b'T').await;
        connection.simple_in("COMMIT", "COMMIT", b'I').await;
        assert_eq!(connection.context.keyset_identifier(), None);
    }

    #[tokio::test]
    async fn rollback_to_savepoint_undoes_only_later_changes() {
        let mut connection = connection().await;
        connection.simple_in("BEGIN", "BEGIN", b'T').await;
        connection
            .simple_in(&set_keyset(KEYSET_A), "SET", b'T')
            .await;
        connection.simple_in("SAVEPOINT s", "SAVEPOINT", b'T').await;
        connection
            .simple_in(&set_keyset(KEYSET_B), "SET", b'T')
            .await;
        connection
            .simple_in("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET", b'T')
            .await;
        connection
            .simple_in("ROLLBACK TO SAVEPOINT s", "ROLLBACK", b'T')
            .await;

        assert_eq!(connection.context.keyset_identifier(), keyset(KEYSET_A));
        assert!(!connection.context.unsafe_disable_mapping());

        connection
            .simple_in("RELEASE SAVEPOINT s", "RELEASE", b'T')
            .await;
        connection.simple_in("COMMIT", "COMMIT", b'I').await;
        assert_eq!(connection.context.keyset_identifier(), keyset(KEYSET_A));
    }

    #[tokio::test]
    async fn a_failed_transaction_restores_the_settings() {
        // ROLLBACK, or a COMMIT that PostgreSQL turns into a ROLLBACK
        for end in [("ROLLBACK", "ROLLBACK"), ("COMMIT", "ROLLBACK")] {
            let mut connection = connection().await;
            connection.simple(&set_keyset(KEYSET_A), "SET").await;

            connection.simple_in("BEGIN", "BEGIN", b'T').await;
            connection
                .simple_in(&set_keyset(KEYSET_B), "SET", b'T')
                .await;
            connection.failed_in("SELECT 1/0", b'E').await;
            connection.simple_in(end.0, end.1, b'I').await;

            assert_eq!(
                connection.context.keyset_identifier(),
                keyset(KEYSET_A),
                "{end:?}"
            );
        }
    }

    #[tokio::test]
    async fn set_local_ends_with_the_transaction() {
        for end in ["COMMIT", "ROLLBACK"] {
            let mut connection = connection().await;
            connection.simple(&set_keyset(KEYSET_A), "SET").await;

            connection.simple_in("BEGIN", "BEGIN", b'T').await;
            connection
                .simple_in(
                    &format!("SET LOCAL CIPHERSTASH.KEYSET_ID = '{KEYSET_B}'"),
                    "SET",
                    b'T',
                )
                .await;
            connection
                .simple_in(
                    "SET LOCAL CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
                    "SET",
                    b'T',
                )
                .await;
            assert_eq!(connection.context.keyset_identifier(), keyset(KEYSET_B));
            assert!(connection.context.unsafe_disable_mapping());
            connection.simple_in(end, end, b'I').await;

            assert_eq!(
                connection.context.keyset_identifier(),
                keyset(KEYSET_A),
                "{end}"
            );
            assert!(!connection.context.unsafe_disable_mapping(), "{end}");
        }
    }

    #[tokio::test]
    async fn set_local_outside_a_transaction_ends_with_the_statement() {
        let mut connection = connection().await;
        connection
            .simple(
                &format!("SET LOCAL CIPHERSTASH.KEYSET_ID = '{KEYSET_A}'"),
                "SET",
            )
            .await;
        assert_eq!(connection.context.keyset_identifier(), None);
    }

    #[tokio::test]
    async fn a_rolled_back_set_is_undone_in_the_extended_protocol() {
        let mut connection = connection().await;
        connection.extended(&set_keyset(KEYSET_A), "SET").await;

        connection.extended_in("BEGIN", "BEGIN", b'T').await;
        connection
            .extended_in(&set_keyset(KEYSET_B), "SET", b'T')
            .await;
        connection.extended_in("RESET ALL", "RESET", b'T').await;
        connection.extended_in("ROLLBACK", "ROLLBACK", b'I').await;

        assert_eq!(connection.context.keyset_identifier(), keyset(KEYSET_A));
    }

    #[tokio::test]
    async fn a_prepared_set_in_a_rolled_back_transaction_is_undone() {
        let mut connection = connection().await;
        connection
            .prepare("disable", "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true")
            .await;

        connection.simple_in("BEGIN", "BEGIN", b'T').await;
        connection.client(bind("", "disable")).await;
        connection.client(execute("")).await;
        connection.client(sync()).await;
        connection.server(bind_complete()).await;
        connection.server(command_complete("SET")).await;
        connection.server(ready(b'T')).await;
        assert!(connection.context.unsafe_disable_mapping());
        connection.simple_in("ROLLBACK", "ROLLBACK", b'I').await;

        assert!(!connection.context.unsafe_disable_mapping());
    }

    #[tokio::test]
    async fn a_failed_extended_batch_rolls_back_its_implicit_transaction() {
        let mut connection = connection().await;
        connection.simple(&set_keyset(KEYSET_A), "SET").await;

        // The SET succeeds, but a later Execute in the same batch fails, so PostgreSQL rolls
        // back the batch's implicit transaction
        connection.client(parse("", &set_keyset(KEYSET_B))).await;
        connection.client(bind("", "")).await;
        connection.client(execute("")).await;
        connection.client(parse("", "SELECT 1/0")).await;
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

        assert_eq!(connection.context.keyset_identifier(), keyset(KEYSET_A));
    }

    #[tokio::test]
    async fn a_set_and_its_rollback_in_one_query() {
        let mut connection = connection().await;
        connection.simple(&set_keyset(KEYSET_A), "SET").await;

        connection
            .client(query(&format!("BEGIN; {}; ROLLBACK", set_keyset(KEYSET_B))))
            .await;
        connection.server(command_complete("BEGIN")).await;
        connection.server(command_complete("SET")).await;
        connection.server(command_complete("ROLLBACK")).await;
        connection.server(ready(b'I')).await;

        assert_eq!(connection.context.keyset_identifier(), keyset(KEYSET_A));
    }

    #[tokio::test]
    async fn statements_before_a_set_keep_the_set_in_step() {
        let mut connection = connection().await;

        // Each statement is confirmed in order, so the SELECT's CommandComplete is not taken
        // for the SET
        connection
            .client(query(&format!("SELECT 1; {}", set_keyset(KEYSET_A))))
            .await;
        connection.server(command_complete("SELECT 1")).await;
        assert_eq!(connection.context.keyset_identifier(), None);
        connection.server(command_complete("SET")).await;
        connection.server(ready(b'I')).await;

        assert_eq!(connection.context.keyset_identifier(), keyset(KEYSET_A));
    }

    #[tokio::test]
    async fn an_unknown_cipherstash_setting_is_refused() {
        let mut connection = connection().await;
        connection
            .client(query("SET CIPHERSTASH.UNKNOWN = 'value'"))
            .await;
        assert_eq!(connection.errors_sent_to_client(), 1);
        connection
            .client(parse("unknown", "RESET cipherstash.unknown"))
            .await;
        assert_eq!(connection.errors_sent_to_client(), 1);
    }

    #[tokio::test]
    async fn a_prepared_set_local_ends_with_the_transaction() {
        let mut connection = connection().await;
        connection
            .prepare(
                "local",
                &format!("SET LOCAL CIPHERSTASH.KEYSET_ID = '{KEYSET_A}'"),
            )
            .await;

        connection.extended_in("BEGIN", "BEGIN", b'T').await;
        connection.client(bind("", "local")).await;
        connection.client(execute("")).await;
        connection.client(sync()).await;
        connection.server(bind_complete()).await;
        connection.server(command_complete("SET")).await;
        connection.server(ready(b'T')).await;
        assert_eq!(connection.context.keyset_identifier(), keyset(KEYSET_A));
        connection.extended_in("COMMIT", "COMMIT", b'I').await;

        assert_eq!(connection.context.keyset_identifier(), None);
    }

    #[tokio::test]
    async fn statements_sent_before_a_change_is_confirmed_use_the_change() {
        // With a default keyset, a keyset SET is refused while mapping is enabled and ignored
        // while it is disabled
        let mut connection = Connection::started().await;
        connection
            .client(query("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true"))
            .await;
        assert!(!connection.context.unsafe_disable_mapping());
        connection.client(query(&set_keyset(KEYSET_A))).await;
        assert_eq!(connection.errors_sent_to_client(), 0);

        // Once the transaction that disabled mapping rolls back, the keyset SET is refused
        let mut connection = Connection::started().await;
        connection.simple_in("BEGIN", "BEGIN", b'T').await;
        connection
            .simple_in("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET", b'T')
            .await;
        connection.client(query("ROLLBACK")).await;
        connection.client(query(&set_keyset(KEYSET_A))).await;
        assert_eq!(connection.errors_sent_to_client(), 1);
    }

    #[tokio::test]
    async fn a_set_and_a_dependent_statement_in_one_query_are_both_forwarded() {
        let mut connection = connection().await;
        connection
            .client(query(&format!(
                "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true; SELECT 1; {}",
                set_keyset(KEYSET_A)
            )))
            .await;
        connection.server(command_complete("SET")).await;
        connection.server(command_complete("SELECT 1")).await;
        connection.server(command_complete("SET")).await;
        connection.server(ready(b'I')).await;

        assert!(connection.context.unsafe_disable_mapping());
        assert_eq!(connection.context.keyset_identifier(), keyset(KEYSET_A));
        assert_eq!(connection.context.state_sizes().pending_settings, 0);
    }

    /// Parse `sql` as statement `name` and Sync, leaving `status`
    async fn prepare_in(connection: &mut Connection, name: &str, sql: &str, status: u8) {
        connection.client(parse(name, sql)).await;
        connection.client(sync()).await;
        connection.server(parse_complete()).await;
        connection.server(ready(status)).await;
    }

    #[tokio::test]
    async fn a_statement_prepared_while_mapping_was_disabled_is_refused_after_a_rollback() {
        let mut connection = connection().await;
        connection.simple_in("BEGIN", "BEGIN", b'T').await;
        connection
            .simple_in("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET", b'T')
            .await;
        prepare_in(&mut connection, "s", "SELECT 1", b'T').await;
        connection.simple_in("ROLLBACK", "ROLLBACK", b'I').await;
        assert_eq!(connection.errors_sent_to_client(), 0);

        // The statement was not mapped, so binding it now would forward plaintext
        connection.client(bind("", "s")).await;
        assert_eq!(connection.errors_sent_to_client(), 1);
    }

    #[tokio::test]
    async fn a_statement_prepared_while_mapping_was_enabled_is_refused_once_it_is_disabled() {
        let mut connection = connection().await;
        prepare_in(&mut connection, "s", "SELECT 1", b'I').await;
        connection
            .simple("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET")
            .await;

        connection.client(bind("", "s")).await;
        assert_eq!(connection.errors_sent_to_client(), 1);
    }

    #[tokio::test]
    async fn a_portal_bound_before_mapping_is_disabled_is_refused_at_execute() {
        let mut connection = connection().await;
        prepare_in(&mut connection, "s", "SELECT 1", b'T').await;
        connection.client(bind("p", "s")).await;
        connection.client(sync()).await;
        connection.server(bind_complete()).await;
        connection.server(ready(b'T')).await;
        connection
            .simple_in("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET", b'T')
            .await;
        assert_eq!(connection.errors_sent_to_client(), 0);

        connection.client(execute("p")).await;
        assert_eq!(connection.errors_sent_to_client(), 1);
        assert_eq!(connection.context.state_sizes().pending_settings, 0);
    }

    #[tokio::test]
    async fn a_statement_prepared_again_after_the_change_is_accepted() {
        let mut connection = connection().await;
        prepare_in(&mut connection, "s", "SELECT 1", b'I').await;
        connection
            .simple("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET")
            .await;
        prepare_in(&mut connection, "s", "SELECT 1", b'I').await;

        connection.client(bind("", "s")).await;
        connection.client(execute("")).await;
        assert_eq!(connection.errors_sent_to_client(), 0);
    }

    #[tokio::test]
    async fn a_closed_statement_and_its_portal_retain_nothing() {
        let mut connection = connection().await;
        prepare_in(&mut connection, "s", "SELECT 1", b'I').await;
        connection.client(bind("p", "s")).await;
        connection.client(close_statement("s")).await;
        connection.client(sync()).await;
        connection.server(bind_complete()).await;
        connection.server(close_complete()).await;
        connection.server(ready(b'I')).await;

        let sizes = connection.context.state_sizes();
        assert_eq!(sizes.prepared_settings, 0);
        assert_eq!(sizes.bound_settings, 0);
    }

    #[tokio::test]
    async fn the_projection_follows_responses_to_a_pipeline() {
        let mut connection = connection().await;
        connection.client(parse("", "SELECT 1")).await;
        connection.client(bind("", "")).await;
        connection.client(execute("")).await;
        connection
            .client(parse("", "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true"))
            .await;
        connection.client(bind("", "")).await;
        connection.client(execute("")).await;
        connection.client(sync()).await;
        assert!(connection
            .context
            .projected_settings()
            .unsafe_disable_mapping());

        connection.server(parse_complete()).await;
        connection.server(bind_complete()).await;
        connection.server(command_complete("SELECT 1")).await;
        assert!(connection
            .context
            .projected_settings()
            .unsafe_disable_mapping());

        // PostgreSQL skips the SET
        connection.server(parse_complete()).await;
        connection.server(bind_complete()).await;
        connection.server(error()).await;
        assert!(!connection
            .context
            .projected_settings()
            .unsafe_disable_mapping());
        connection.server(ready(b'I')).await;
        assert!(!connection
            .context
            .projected_settings()
            .unsafe_disable_mapping());
        assert_eq!(connection.context.state_sizes().pending_settings, 0);
    }
}
