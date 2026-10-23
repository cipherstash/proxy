//! Tests that drive the frontend and backend of one connection together with the messages a
//! client and PostgreSQL exchange.

use super::{backend::Backend, frontend::Frontend, Column, Context, KeysetIdentifier};
use crate::{
    config::TandemConfig,
    error::Error,
    proxy::{EncryptConfig, EncryptionService},
};
use bytes::{BufMut, BytesMut};
use eql_mapper::{EqlTraits, Schema, Table};
use sqltk::parser::ast::Ident;
use std::sync::{Arc, Mutex};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt, DuplexStream, ReadHalf},
    sync::mpsc,
};

/// Records the keyset of each encryption and decryption, and refuses it, as it has no keys.
/// With `encrypts`, an encryption returns stub ciphertexts instead.
#[derive(Clone, Default)]
struct TestService {
    encrypted_with: Arc<Mutex<Vec<Option<KeysetIdentifier>>>>,
    decrypted_with: Arc<Mutex<Vec<Option<KeysetIdentifier>>>>,
    encrypts: Arc<std::sync::atomic::AtomicBool>,
}

#[async_trait::async_trait]
impl EncryptionService for TestService {
    async fn encrypt(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        plaintexts: Vec<Option<cipherstash_client::encryption::Plaintext>>,
        columns: &[Option<Column>],
    ) -> Result<Vec<Option<crate::EqlCiphertext>>, Error> {
        if plaintexts.iter().all(Option::is_none) {
            return Ok(vec![None; plaintexts.len()]);
        }
        self.encrypted_with.lock().unwrap().push(keyset_id);
        if self.encrypts.load(std::sync::atomic::Ordering::Relaxed) {
            // A ciphertext with no encrypted record, which is enough for Proxy to rewrite a
            // statement
            return Ok(plaintexts
                .iter()
                .zip(columns)
                .map(|(plaintext, column)| {
                    plaintext.as_ref()?;
                    let identifier = &column.as_ref()?.identifier;
                    Some(
                        serde_json::from_value(serde_json::json!({
                            "i": { "t": identifier.table, "c": identifier.column },
                            "v": 2,
                        }))
                        .unwrap(),
                    )
                })
                .collect());
        }
        Err(crate::error::EncryptError::ColumnCouldNotBeEncrypted {
            table: "users".to_string(),
            column: "email".to_string(),
        }
        .into())
    }

    async fn decrypt(
        &self,
        keyset_id: Option<KeysetIdentifier>,
        ciphertexts: Vec<Option<crate::EqlCiphertext>>,
    ) -> Result<Vec<Option<cipherstash_client::encryption::Plaintext>>, Error> {
        if ciphertexts.iter().all(Option::is_none) {
            return Ok(vec![None; ciphertexts.len()]);
        }
        self.decrypted_with.lock().unwrap().push(keyset_id.clone());
        Err(crate::error::EncryptError::CouldNotDecryptDataForKeyset {
            keyset_id: format!("{keyset_id:?}"),
        }
        .into())
    }
}

/// One client connection through Proxy.
///
/// [`Connection::client`] sends a message from the client through the frontend, and
/// [`Connection::server`] sends a message from PostgreSQL through the backend.
struct Connection {
    client: DuplexStream,
    frontend: Frontend<ReadHalf<DuplexStream>, DuplexStream, TestService>,
    /// What the frontend forwards to PostgreSQL
    to_server: DuplexStream,
    server: DuplexStream,
    backend: Backend<ReadHalf<DuplexStream>, TestService>,
    to_client: mpsc::UnboundedReceiver<BytesMut>,
    context: Context<TestService>,
    /// The keyset of each encryption so far
    encrypted_with: Arc<Mutex<Vec<Option<KeysetIdentifier>>>>,
    /// The keyset of each decryption so far
    decrypted_with: Arc<Mutex<Vec<Option<KeysetIdentifier>>>>,
    /// Set to have encryptions return stub ciphertexts
    encrypts: Arc<std::sync::atomic::AtomicBool>,
}

impl Connection {
    fn new() -> Self {
        Self::with_config(TandemConfig::for_testing())
    }

    fn with_config(config: TandemConfig) -> Self {
        Self::with_schema(config, EncryptConfig::default(), Schema::new("public"))
    }

    /// A started connection to a Proxy with no default keyset that encrypts `users.email`
    async fn encrypted() -> Self {
        let mut config = TandemConfig::for_testing();
        config.encrypt.default_keyset_id = None;
        let encrypt_config = EncryptConfig::from_json(serde_json::json!({
            "v": 1,
            "tables": { "users": { "email": {} } }
        }));
        // Quoted column names, as Proxy loads them from the database
        let mut users = Table::new(Ident::new("users"));
        users.add_column(Arc::new(eql_mapper::Column::native(Ident::with_quote(
            '"', "id",
        ))));
        users.add_column(Arc::new(eql_mapper::Column::eql(
            Ident::with_quote('"', "email"),
            EqlTraits::all(),
        )));
        let mut schema = Schema::new("public");
        schema.add_table(users);

        let mut connection = Self::with_schema(config, encrypt_config, schema);
        connection.server(ready(b'I')).await;
        connection
    }

    fn with_schema(config: TandemConfig, encrypt_config: EncryptConfig, schema: Schema) -> Self {
        let (reload_sender, _) = mpsc::unbounded_channel();
        let service = TestService::default();
        let encrypted_with = service.encrypted_with.clone();
        let decrypted_with = service.decrypted_with.clone();
        let encrypts = service.encrypts.clone();
        let context = Context::new(
            1,
            Arc::new(config),
            Arc::new(encrypt_config),
            Arc::new(schema),
            service,
            reload_sender,
        );

        let (client, client_reader) = tokio::io::duplex(64 * 1024);
        let (client_reader, _) = tokio::io::split(client_reader);
        let (server, server_reader) = tokio::io::duplex(64 * 1024);
        let (server_reader, _) = tokio::io::split(server_reader);
        let (client_sender, to_client) = mpsc::unbounded_channel();
        let (server_writer, to_server) = tokio::io::duplex(64 * 1024);

        Self {
            client,
            frontend: Frontend::new(
                client_reader,
                client_sender.clone(),
                server_writer,
                context.clone(),
            ),
            to_server,
            server,
            backend: Backend::new(client_sender, server_reader, context.clone()),
            to_client,
            context,
            encrypted_with,
            decrypted_with,
            encrypts,
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
        tokio::time::timeout(std::time::Duration::from_secs(1), self.frontend.rewrite())
            .await
            .expect("the frontend is waiting for PostgreSQL")
            .unwrap();
    }

    /// The client sends `message`, which the frontend holds until PostgreSQL sends `responses`
    async fn client_after(&mut self, message: BytesMut, responses: Vec<BytesMut>) {
        self.client.write_all(&message).await.unwrap();
        let rewrite = self.frontend.rewrite();
        tokio::pin!(rewrite);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(10), &mut rewrite)
                .await
                .is_err(),
            "the frontend did not wait for PostgreSQL"
        );
        for response in responses {
            self.server.write_all(&response).await.unwrap();
            self.backend.rewrite().await.unwrap();
        }
        rewrite.await.unwrap();
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

    /// The messages forwarded to PostgreSQL since the last call, as (code, body)
    async fn forwarded(&mut self) -> Vec<(u8, Vec<u8>)> {
        let mut bytes = vec![];
        let mut buffer = [0; 4096];
        while let Ok(Ok(read)) = tokio::time::timeout(
            std::time::Duration::from_millis(10),
            self.to_server.read(&mut buffer),
        )
        .await
        {
            if read == 0 {
                break;
            }
            bytes.extend_from_slice(&buffer[..read]);
        }
        let mut messages = vec![];
        let mut rest = &bytes[..];
        while rest.len() >= 5 {
            let len = i32::from_be_bytes(rest[1..5].try_into().unwrap()) as usize;
            messages.push((rest[0], rest[5..1 + len].to_vec()));
            rest = &rest[1 + len..];
        }
        messages
    }

    /// The codes of the messages forwarded to PostgreSQL since the last call
    async fn forwarded_codes(&mut self) -> Vec<u8> {
        self.forwarded()
            .await
            .into_iter()
            .map(|(code, _)| code)
            .collect()
    }

    /// The keyset of each decryption so far
    fn decrypted_with(&self) -> Vec<Option<KeysetIdentifier>> {
        self.decrypted_with.lock().unwrap().clone()
    }

    /// The keyset of each encryption so far
    fn encrypted_with(&self) -> Vec<Option<KeysetIdentifier>> {
        self.encrypted_with.lock().unwrap().clone()
    }

    /// PostgreSQL rejects the message Proxy forwarded in place of a refused one, then sends
    /// `ReadyForQuery` with `transaction_status`
    async fn rejected(&mut self, transaction_status: u8) {
        let forwarded = self.forwarded().await;
        self.server(rejection(&forwarded)).await;
        self.server(ready(transaction_status)).await;
    }

    /// The `ErrorResponse` messages sent to the client so far, as their SQLSTATE codes
    fn error_codes_sent_to_client(&mut self) -> Vec<String> {
        let mut codes = vec![];
        while let Ok(message) = self.to_client.try_recv() {
            if message.first() == Some(&b'E') {
                let fields = &message[5..];
                let code = fields
                    .split(|byte| *byte == 0)
                    .find(|field| field.first() == Some(&b'C'))
                    .map(|field| String::from_utf8_lossy(&field[1..]).into_owned())
                    .unwrap_or_default();
                codes.push(code);
            }
        }
        codes
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

/// A `Bind` of `statement` with one text parameter
fn bind_text(portal: &str, statement: &str, value: &str) -> BytesMut {
    let mut body = vec![];
    cstring(&mut body, portal);
    cstring(&mut body, statement);
    // No parameter formats, so text
    body.extend_from_slice(&0i16.to_be_bytes());
    body.extend_from_slice(&1i16.to_be_bytes());
    body.extend_from_slice(&(value.len() as i32).to_be_bytes());
    body.extend_from_slice(value.as_bytes());
    // No result formats
    body.extend_from_slice(&0i16.to_be_bytes());
    message(b'B', &body)
}

fn execute(portal: &str) -> BytesMut {
    let mut body = vec![];
    cstring(&mut body, portal);
    body.extend_from_slice(&0i32.to_be_bytes());
    message(b'E', &body)
}

/// A `DataRow` of one `eql_v2_encrypted` value for `users.email`, in text format
fn encrypted_email_row() -> BytesMut {
    let value = br#"("{""i"": {""c"": ""email"", ""t"": ""users""}, ""v"": 2}")"#;
    let mut body = vec![];
    body.extend_from_slice(&1i16.to_be_bytes());
    body.extend_from_slice(&(value.len() as i32).to_be_bytes());
    body.extend_from_slice(value);
    message(b'D', &body)
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

/// PostgreSQL's error for the message Proxy forwarded in place of a refused one, among
/// `forwarded`, which names an object that does not exist
fn rejection(forwarded: &[(u8, Vec<u8>)]) -> BytesMut {
    const PREFIX: &[u8] = b"__cipherstash_";
    let name = forwarded
        .iter()
        .find_map(|(_, body)| {
            let start = body.windows(PREFIX.len()).position(|w| w == PREFIX)?;
            Some(String::from_utf8_lossy(&body[start..start + PREFIX.len() + 32]).into_owned())
        })
        .expect("no refused message was forwarded");
    let mut body = b"SERROR\0C26000\0M".to_vec();
    body.extend_from_slice(format!("\"{name}\" does not exist").as_bytes());
    body.extend_from_slice(b"\0\0");
    message(b'E', &body)
}

/// PostgreSQL's error for a statement on a table that does not exist
fn undefined_table() -> BytesMut {
    message(
        b'E',
        b"SERROR\0C42P01\0Mrelation \"missing\" does not exist\0\0",
    )
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
        connection
            .client_after(
                query(KEYSET_ID),
                vec![command_complete("DISCARD ALL"), ready(b'I')],
            )
            .await;
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
            .client_after(
                query("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true"),
                vec![
                    parse_complete(),
                    bind_complete(),
                    command_complete("RESET"),
                    ready(b'I'),
                ],
            )
            .await;
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
        connection
            .client_after(
                bind("", "set_keyset"),
                vec![command_complete("RESET"), ready(b'I')],
            )
            .await;
        connection.client(execute("")).await;
        connection.client(sync()).await;
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
        connection
            .client_after(
                query(OTHER_KEYSET_ID),
                vec![command_complete("RESET"), ready(b'I')],
            )
            .await;
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
        connection.rejected(b'I').await;
        assert_eq!(connection.context.keyset_identifier(), None);
        assert_eq!(connection.errors_sent_to_client(), 1);
    }

    #[tokio::test]
    async fn a_keyset_set_with_a_default_keyset_is_refused_when_prepared() {
        let mut connection = Connection::started().await;
        connection.client(parse("set_keyset", KEYSET_ID)).await;
        connection.client(sync()).await;
        connection.rejected(b'I').await;
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
        connection.client(sync()).await;
        connection.server(bind_complete()).await;
        connection.rejected(b'I').await;
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
        pub(super) async fn simple_in(&mut self, sql: &str, tag: &str, status: u8) {
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
        connection.rejected(b'I').await;
        assert_eq!(connection.errors_sent_to_client(), 1);
        connection
            .client(parse("unknown", "RESET cipherstash.unknown"))
            .await;
        connection.client(sync()).await;
        connection.rejected(b'I').await;
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
    async fn statements_sent_after_a_change_wait_for_it() {
        // With a default keyset, a keyset SET is refused while mapping is enabled and ignored
        // while it is disabled
        let mut connection = Connection::started().await;
        connection
            .client(query("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true"))
            .await;
        assert!(!connection.context.unsafe_disable_mapping());
        connection
            .client_after(
                query(&set_keyset(KEYSET_A)),
                vec![command_complete("SET"), ready(b'I')],
            )
            .await;
        assert_eq!(connection.errors_sent_to_client(), 0);

        // Once the transaction that disabled mapping rolls back, the keyset SET is refused
        let mut connection = Connection::started().await;
        connection.simple_in("BEGIN", "BEGIN", b'T').await;
        connection
            .simple_in("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET", b'T')
            .await;
        connection.client(query("ROLLBACK")).await;
        connection
            .client_after(
                query(&set_keyset(KEYSET_A)),
                vec![command_complete("ROLLBACK"), ready(b'I')],
            )
            .await;
        connection.rejected(b'I').await;
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
        connection.client(sync()).await;
        connection.rejected(b'I').await;
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
        connection.client(sync()).await;
        connection.rejected(b'I').await;
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
        connection.client(sync()).await;
        connection.rejected(b'E').await;
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

/// A statement is mapped under the settings it runs with: once a batch that may change the
/// settings has been sent, Proxy waits for PostgreSQL to answer it before it maps a statement
/// from a later batch, as the change may fail or end with the batch.
mod settings_barrier {
    use super::*;

    const DISABLE_MAPPING: &str = "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true";

    /// A started connection to a Proxy with no default keyset
    async fn connection() -> Connection {
        let mut config = TandemConfig::for_testing();
        config.encrypt.default_keyset_id = None;
        let mut connection = Connection::with_config(config);
        connection.server(ready(b'I')).await;
        connection
    }

    impl Connection {
        /// Bind statement `name` to the unnamed portal, Execute and Sync, with PostgreSQL's
        /// responses
        async fn run_prepared(&mut self, name: &str, tag: &str) {
            self.client(bind("", name)).await;
            self.client(execute("")).await;
            self.client(sync()).await;
            self.server(bind_complete()).await;
            self.server(command_complete(tag)).await;
            self.server(ready(b'I')).await;
        }
    }

    #[tokio::test]
    async fn a_set_in_a_query_that_fails_does_not_apply_to_the_next_batch() {
        let mut connection = connection().await;
        connection.client(query(DISABLE_MAPPING)).await;

        connection
            .client_after(parse("s", "SELECT 1"), vec![error(), ready(b'I')])
            .await;
        connection.client(sync()).await;
        connection.server(parse_complete()).await;
        connection.server(ready(b'I')).await;
        assert!(!connection
            .context
            .projected_settings()
            .unsafe_disable_mapping());

        // `s` was prepared with mapping enabled, as it is now
        connection.errors_sent_to_client();
        connection.run_prepared("s", "SELECT 1").await;
        assert_eq!(connection.errors_sent_to_client(), 0);
    }

    #[tokio::test]
    async fn a_set_local_in_an_implicit_transaction_ends_before_the_next_batch() {
        let mut connection = connection().await;
        connection
            .client(parse(
                "",
                "SET LOCAL CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
            ))
            .await;
        connection.client(bind("", "")).await;
        connection.client(execute("")).await;
        connection.client(sync()).await;

        connection
            .client_after(
                query("SELECT 1"),
                vec![
                    parse_complete(),
                    bind_complete(),
                    command_complete("SET"),
                    ready(b'I'),
                ],
            )
            .await;
        assert!(!connection
            .context
            .projected_settings()
            .unsafe_disable_mapping());
    }

    #[tokio::test]
    async fn a_query_waits_for_a_rollback_sent_before_it() {
        let mut connection = connection().await;
        connection.simple_in("BEGIN", "BEGIN", b'T').await;
        connection.simple_in(DISABLE_MAPPING, "SET", b'T').await;
        connection.client(query("ROLLBACK")).await;

        connection
            .client_after(
                query("SELECT 1"),
                vec![command_complete("ROLLBACK"), ready(b'I')],
            )
            .await;
        assert!(!connection
            .context
            .projected_settings()
            .unsafe_disable_mapping());
    }

    #[tokio::test]
    async fn a_batch_that_cannot_change_the_settings_is_not_waited_for() {
        let mut connection = connection().await;
        connection.client(query("BEGIN")).await;
        connection.client(query("SELECT 1")).await;
        connection.client(parse("", "SAVEPOINT a")).await;
        connection.client(bind("", "")).await;
        connection.client(execute("")).await;
        connection.client(sync()).await;
        connection.client(parse("s", "SELECT 1")).await;
    }

    #[tokio::test]
    async fn a_commit_that_changes_no_setting_is_not_waited_for() {
        let mut connection = connection().await;
        connection.simple_in("BEGIN", "BEGIN", b'T').await;
        connection.simple_in("SELECT 1", "SELECT 1", b'T').await;
        connection.client(query("COMMIT")).await;
        connection.client(query("BEGIN")).await;
        connection.client(query("ROLLBACK")).await;
        connection.client(parse("s", "SELECT 1")).await;
    }

    #[tokio::test]
    async fn a_set_to_the_value_in_effect_is_not_waited_for() {
        let mut connection = connection().await;
        connection.simple(DISABLE_MAPPING, "SET").await;
        connection.client(query(DISABLE_MAPPING)).await;
        connection.client(parse("s", "SELECT 1")).await;
    }

    #[tokio::test]
    async fn a_commit_after_a_set_in_the_transaction_is_waited_for() {
        // If the transaction has failed, PostgreSQL rolls back and undoes the SET
        let mut connection = connection().await;
        connection.simple_in("BEGIN", "BEGIN", b'T').await;
        connection.simple_in(DISABLE_MAPPING, "SET", b'T').await;
        connection.client(query("COMMIT")).await;
        connection
            .client_after(
                parse("s", "SELECT 1"),
                vec![command_complete("COMMIT"), ready(b'I')],
            )
            .await;
        assert!(connection
            .context
            .projected_settings()
            .unsafe_disable_mapping());
    }

    #[tokio::test]
    async fn a_response_to_an_earlier_batch_does_not_apply_a_later_set() {
        let mut connection = connection().await;
        connection.client(query("SELECT 1")).await;
        connection.client(query(DISABLE_MAPPING)).await;

        connection.server(command_complete("SELECT 1")).await;
        assert!(!connection.context.unsafe_disable_mapping());
        connection.server(ready(b'I')).await;
        connection.server(command_complete("SET")).await;
        connection.server(ready(b'I')).await;
        assert!(connection.context.unsafe_disable_mapping());
    }

    #[tokio::test]
    async fn messages_in_the_batch_that_changes_the_settings_are_not_held() {
        let mut connection = connection().await;
        connection.client(parse("", DISABLE_MAPPING)).await;
        connection.client(bind("", "")).await;
        connection.client(execute("")).await;
        connection.client(parse("s", "SELECT 1")).await;
        connection.client(sync()).await;
        assert!(connection
            .context
            .projected_settings()
            .unsafe_disable_mapping());
    }
}

/// With an encrypted column, Proxy encrypts under the settings each statement runs with.
mod encrypted {
    use super::*;
    use cipherstash_client::IdentifiedBy;

    const KEYSET_A: &str = "7e5ea5a6-4e1c-4f4d-8a9c-9f0b4f6a0c11";

    fn keyset_a() -> Option<KeysetIdentifier> {
        Some(KeysetIdentifier(IdentifiedBy::Uuid(
            KEYSET_A.parse().unwrap(),
        )))
    }

    #[tokio::test]
    async fn a_keyset_set_earlier_in_a_query_encrypts_the_values_after_it() {
        let mut connection = Connection::encrypted().await;
        connection
            .client(query(&format!(
                "SET CIPHERSTASH.KEYSET_ID = '{KEYSET_A}'; \
                 INSERT INTO users (email) VALUES ('a@example.com')"
            )))
            .await;
        assert_eq!(connection.encrypted_with(), vec![keyset_a()]);
    }

    #[tokio::test]
    async fn mapping_disabled_earlier_in_a_query_leaves_the_values_after_it() {
        let mut connection = Connection::encrypted().await;
        connection
            .client(query(
                "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true; \
                 INSERT INTO users (email) VALUES ('a@example.com')",
            ))
            .await;
        assert_eq!(connection.encrypted_with(), vec![]);
        assert_eq!(connection.forwarded_codes().await, vec![b'Q']);
        assert_eq!(connection.errors_sent_to_client(), 0);
    }

    #[tokio::test]
    async fn a_keyset_set_earlier_in_a_batch_encrypts_the_parameters_after_it() {
        let mut connection = Connection::encrypted().await;
        connection
            .client(parse(
                "",
                &format!("SET CIPHERSTASH.KEYSET_ID = '{KEYSET_A}'"),
            ))
            .await;
        connection.client(bind("", "")).await;
        connection.client(execute("")).await;
        connection
            .client(parse("", "INSERT INTO users (email) VALUES ($1)"))
            .await;
        connection.client(bind_text("", "", "a@example.com")).await;
        assert_eq!(connection.encrypted_with(), vec![keyset_a()]);
    }

    #[tokio::test]
    async fn rows_are_decrypted_under_the_keyset_postgresql_has_confirmed() {
        const KEYSET_B: &str = "0b9f3e2c-1d4a-4c6e-9f8a-2b7d5e1c3a90";
        let mut connection = Connection::encrypted().await;
        for sql in [
            format!("SET CIPHERSTASH.KEYSET_ID = '{KEYSET_A}'"),
            "SELECT email FROM users".to_string(),
            format!("SET CIPHERSTASH.KEYSET_ID = '{KEYSET_B}'"),
        ] {
            connection.client(parse("", &sql)).await;
            connection.client(bind("", "")).await;
            connection.client(execute("")).await;
        }
        connection.client(sync()).await;

        connection.server(parse_complete()).await;
        connection.server(bind_complete()).await;
        connection.server(command_complete("SET")).await;
        connection.server(parse_complete()).await;
        connection.server(bind_complete()).await;
        connection.server(encrypted_email_row()).await;
        connection.server(command_complete("SELECT 1")).await;
        // The second SET is still to run
        assert_eq!(connection.decrypted_with(), vec![keyset_a()]);
    }

    #[tokio::test]
    async fn a_refused_bind_gets_proxys_error_in_place_of_postgresqls() {
        let mut connection = Connection::encrypted().await;
        connection
            .simple(&format!("SET CIPHERSTASH.KEYSET_ID = '{KEYSET_A}'"), "SET")
            .await;
        connection
            .prepare("s", "INSERT INTO users (email) VALUES ($1)")
            .await;
        connection.forwarded().await;
        connection.errors_sent_to_client();

        // The test service cannot encrypt, so Proxy refuses the Bind
        connection.client(bind_text("", "s", "a@example.com")).await;
        connection.client(execute("")).await;
        connection.client(sync()).await;
        let forwarded = connection.forwarded().await;
        let codes = forwarded.iter().map(|(code, _)| *code).collect::<Vec<_>>();
        assert_eq!(codes, vec![b'B', b'S']);

        connection.server(rejection(&forwarded)).await;
        connection.server(ready(b'I')).await;
        let codes = connection.error_codes_sent_to_client();
        assert_eq!(codes.len(), 1);
        assert_ne!(codes[0], "25P02");
    }
}

/// A message Proxy refuses still fails the batch, or the transaction, in PostgreSQL: Proxy
/// forwards a message PostgreSQL rejects in its place, and the Sync, and sends the client its own
/// error in place of PostgreSQL's.
mod refusals {
    use super::*;

    const UNDEFINED_OBJECT_OR_NAME: [&str; 2] = ["26000", "34000"];

    /// A started connection to a Proxy with no default keyset, with statement `s` prepared while
    /// mapping was enabled and mapping now disabled
    async fn connection_with_stale_statement() -> Connection {
        let mut config = TandemConfig::for_testing();
        config.encrypt.default_keyset_id = None;
        let mut connection = Connection::with_config(config);
        connection.server(ready(b'I')).await;
        connection.prepare("s", "SELECT 1").await;
        connection
            .simple("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", "SET")
            .await;
        connection.forwarded().await;
        connection.errors_sent_to_client();
        connection
    }

    /// The name a forwarded Bind binds, or a forwarded Execute executes
    fn target(body: &[u8], skip_portal: bool) -> String {
        let mut fields = body.split(|byte| *byte == 0);
        if skip_portal {
            fields.next();
        }
        String::from_utf8_lossy(fields.next().unwrap()).into_owned()
    }

    #[tokio::test]
    async fn a_refused_bind_is_forwarded_as_a_bind_postgresql_rejects() {
        let mut connection = connection_with_stale_statement().await;
        connection.client(bind("", "s")).await;
        connection.client(execute("")).await;
        connection.client(sync()).await;

        let forwarded = connection.forwarded().await;
        let codes = forwarded.iter().map(|(code, _)| *code).collect::<Vec<_>>();
        assert_eq!(codes, vec![b'B', b'S']);
        assert!(target(&forwarded[0].1, true).starts_with("__cipherstash_"));
        // The client gets Proxy's error once PostgreSQL rejects the Bind, then PostgreSQL's
        // ReadyForQuery
        assert_eq!(
            connection.error_codes_sent_to_client(),
            Vec::<String>::new()
        );

        connection.server(rejection(&forwarded)).await;
        connection.server(ready(b'I')).await;
        assert_eq!(connection.error_codes_sent_to_client(), vec!["26000"]);
        assert_eq!(connection.context.state_sizes().pending_settings, 0);
    }

    #[tokio::test]
    async fn a_refused_execute_is_forwarded_as_an_execute_postgresql_rejects() {
        let mut connection = Connection::started().await;
        connection.client(query("BEGIN")).await;
        connection.server(command_complete("BEGIN")).await;
        connection.server(ready(b'T')).await;
        connection.prepare("s", "SELECT 1").await;
        connection.client(bind("p", "s")).await;
        connection.client(sync()).await;
        connection.server(bind_complete()).await;
        connection.server(ready(b'T')).await;
        connection
            .client(query("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true"))
            .await;
        connection.server(command_complete("SET")).await;
        connection.server(ready(b'T')).await;
        connection.forwarded().await;
        connection.errors_sent_to_client();

        connection.client(execute("p")).await;
        connection.client(sync()).await;

        let forwarded = connection.forwarded().await;
        let codes = forwarded.iter().map(|(code, _)| *code).collect::<Vec<_>>();
        assert_eq!(codes, vec![b'E', b'S']);
        assert!(target(&forwarded[0].1, false).starts_with("__cipherstash_"));

        // PostgreSQL fails the transaction, so the client's COMMIT rolls back
        connection.server(rejection(&forwarded)).await;
        connection.server(ready(b'E')).await;
        assert_eq!(connection.error_codes_sent_to_client(), vec!["26000"]);
    }

    #[tokio::test]
    async fn a_refused_parse_is_forwarded_as_a_parse_postgresql_rejects() {
        let mut connection = Connection::started().await;
        connection.forwarded().await;
        connection
            .client(parse("", "SET CIPHERSTASH.UNKNOWN_SETTING = 1"))
            .await;
        connection.client(sync()).await;

        let forwarded = connection.forwarded().await;
        let codes = forwarded.iter().map(|(code, _)| *code).collect::<Vec<_>>();
        assert_eq!(codes, vec![b'P', b'S']);
        connection.server(rejection(&forwarded)).await;
        connection.server(ready(b'I')).await;
        let codes = connection.error_codes_sent_to_client();
        assert_eq!(codes.len(), 1);
        assert!(!UNDEFINED_OBJECT_OR_NAME.contains(&codes[0].as_str()));
    }

    #[tokio::test]
    async fn a_refused_query_is_forwarded_as_a_query_postgresql_rejects() {
        let mut connection = Connection::started().await;
        connection.client(query("BEGIN")).await;
        connection.server(command_complete("BEGIN")).await;
        connection.server(ready(b'T')).await;
        connection.forwarded().await;
        connection.errors_sent_to_client();

        connection
            .client(query("SET CIPHERSTASH.UNKNOWN_SETTING = 1"))
            .await;

        // PostgreSQL answers, and fails the transaction, rather than Proxy answering for it
        let forwarded = connection.forwarded().await;
        let codes = forwarded.iter().map(|(code, _)| *code).collect::<Vec<_>>();
        assert_eq!(codes, vec![b'Q']);
        assert_eq!(
            connection.error_codes_sent_to_client(),
            Vec::<String>::new()
        );
        connection.server(rejection(&forwarded)).await;
        connection.server(ready(b'E')).await;
        let codes = connection.error_codes_sent_to_client();
        assert_eq!(codes.len(), 1);
        assert!(!UNDEFINED_OBJECT_OR_NAME.contains(&codes[0].as_str()));
    }

    #[tokio::test]
    async fn an_earlier_error_in_the_batch_reaches_the_client_in_place_of_the_refusal() {
        let mut connection = connection_with_stale_statement().await;
        connection.client(parse("q", "SELECT * FROM missing")).await;
        connection.client(bind("", "s")).await;
        connection.client(execute("")).await;
        connection.client(sync()).await;
        assert_eq!(connection.forwarded_codes().await, vec![b'P', b'B', b'S']);

        // PostgreSQL fails the Parse, and skips the rest of the batch
        connection.server(undefined_table()).await;
        connection.server(ready(b'I')).await;
        assert_eq!(connection.error_codes_sent_to_client(), vec!["42P01"]);
        assert_eq!(connection.context.state_sizes().refusals, 0);
    }
}

/// A `COPY ... FROM STDIN` into a table with no encrypted columns is forwarded with its copy
/// data, and one into an encrypted column is refused, as Proxy does not encrypt copy data.
mod copy_from_stdin {
    use super::*;

    fn copy_in_response() -> BytesMut {
        // Text format, one text column
        message(b'G', &[0, 0, 1, 0, 0])
    }

    fn copy_data(row: &str) -> BytesMut {
        message(b'd', row.as_bytes())
    }

    fn copy_done() -> BytesMut {
        message(b'c', b"")
    }

    /// Check that Proxy refused what the client sent: it forwarded only a message PostgreSQL
    /// rejects in its place, and none of the client's SQL. PostgreSQL then rejects it, and the
    /// client gets Proxy's errors, which are returned.
    async fn refused(connection: &mut Connection) -> Vec<String> {
        connection.errors_sent_to_client();
        let forwarded = connection.forwarded().await;
        for (_, body) in &forwarded {
            let body = String::from_utf8_lossy(body);
            assert!(!body.contains("COPY") && !body.contains("alice"), "{body}");
        }
        connection.server(rejection(&forwarded)).await;
        connection.server(ready(b'I')).await;

        let mut errors = vec![];
        while let Ok(message) = connection.to_client.try_recv() {
            if message.first() == Some(&b'E') {
                let field = message[5..]
                    .split(|byte| *byte == 0)
                    .find(|field| field.first() == Some(&b'M'))
                    .unwrap();
                errors.push(String::from_utf8_lossy(&field[1..]).into_owned());
            }
        }
        errors
    }

    /// The Query was forwarded unchanged
    async fn forwarded_unchanged(connection: &mut Connection, sql: &str) {
        assert_eq!(
            connection.forwarded().await,
            vec![(b'Q', format!("{sql}\0").into_bytes())]
        );
    }

    #[tokio::test]
    async fn a_simple_query_copy_into_a_plaintext_table_is_forwarded_with_its_data() {
        let mut connection = Connection::started().await;
        let sql = "COPY users (email) FROM STDIN";

        connection.client(query(sql)).await;
        connection.server(copy_in_response()).await;
        connection.client(copy_data("alice@example.com\n")).await;
        connection.client(copy_done()).await;
        connection.server(command_complete("COPY 1")).await;
        connection.server(ready(b'I')).await;

        assert_eq!(
            connection.forwarded().await,
            vec![
                (b'Q', format!("{sql}\0").into_bytes()),
                (b'd', b"alice@example.com\n".to_vec()),
                (b'c', vec![]),
            ]
        );
        assert_eq!(connection.errors_sent_to_client(), 0);
    }

    #[tokio::test]
    async fn a_simple_query_copy_into_an_encrypted_column_is_refused() {
        let mut connection = Connection::encrypted().await;

        connection
            .client(query("COPY users (id, email) FROM STDIN"))
            .await;

        let errors = refused(&mut connection).await;
        assert_eq!(errors.len(), 1);
        assert!(errors[0]
            .starts_with("COPY into encrypted column 'email' of table 'users' is not supported"));
    }

    /// The SQL parser would read the statements after a `COPY ... FROM STDIN` as its copy data
    #[tokio::test]
    async fn a_statement_after_a_copy_in_one_simple_query_is_not_forwarded_as_plaintext() {
        let mut connection = Connection::encrypted().await;

        connection
            .client(query(
                "COPY users (id) FROM STDIN; INSERT INTO users (email) VALUES ('alice@example.com')",
            ))
            .await;

        // The test encryption service refuses to encrypt the INSERT
        assert_eq!(refused(&mut connection).await.len(), 1);
        assert_eq!(connection.encrypted_with().len(), 1);
    }

    /// Proxy writes out each statement again when it rewrites a Query to encrypt it
    #[tokio::test]
    async fn a_copy_in_one_simple_query_with_a_statement_that_is_encrypted_is_refused() {
        let mut connection = Connection::encrypted().await;
        connection
            .encrypts
            .store(true, std::sync::atomic::Ordering::Relaxed);

        connection
            .client(query(
                "INSERT INTO users (email) VALUES ('alice@example.com'); COPY users (id) FROM STDIN",
            ))
            .await;

        let errors = refused(&mut connection).await;
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("COPY must be sent in a query of its own"));
    }

    /// A refused Query leaves nothing behind to describe the results of the next one
    #[tokio::test]
    async fn the_query_after_a_refused_copy_with_an_encrypted_statement_is_not_decrypted() {
        let mut connection = Connection::encrypted().await;
        connection
            .encrypts
            .store(true, std::sync::atomic::Ordering::Relaxed);
        connection
            .client(query(
                "INSERT INTO users (email) VALUES ('alice@example.com') RETURNING email; COPY users (id) FROM STDIN",
            ))
            .await;
        refused(&mut connection).await;

        connection.client(query("SELECT 'hello'")).await;
        let mut body = vec![];
        body.extend_from_slice(&1i16.to_be_bytes());
        body.extend_from_slice(&5i32.to_be_bytes());
        body.extend_from_slice(b"hello");
        let row = message(b'D', &body);
        connection.server(row.clone()).await;
        connection.server(command_complete("SELECT 1")).await;
        connection.server(ready(b'I')).await;

        assert_eq!(connection.decrypted_with(), vec![]);
        assert_eq!(connection.to_client.try_recv().unwrap(), row);
    }

    #[tokio::test]
    async fn an_extended_copy_into_a_plaintext_table_is_forwarded_with_its_data() {
        let mut connection = Connection::started().await;
        let sql = "COPY users (email) FROM STDIN";

        connection.client(parse("", sql)).await;
        connection.client(bind("", "")).await;
        connection.client(execute("")).await;
        connection.client(sync()).await;
        connection.server(parse_complete()).await;
        connection.server(bind_complete()).await;
        connection.server(copy_in_response()).await;
        connection.client(copy_data("alice@example.com\n")).await;
        connection.client(copy_done()).await;
        // PostgreSQL ignores the Sync sent during the copy
        connection.client(sync()).await;
        connection.server(command_complete("COPY 1")).await;
        connection.server(ready(b'I')).await;

        assert_eq!(connection.forwarded_codes().await, b"PBESdcS".to_vec());
        assert_eq!(connection.errors_sent_to_client(), 0);

        // The next batch is matched to its own ReadyForQuery
        connection.simple("SELECT 1", "SELECT 1").await;
        assert_eq!(connection.errors_sent_to_client(), 0);
    }

    #[tokio::test]
    async fn an_extended_copy_into_an_encrypted_column_is_refused() {
        let mut connection = Connection::encrypted().await;

        connection.client(parse("", "COPY users FROM STDIN")).await;
        connection.client(bind("", "")).await;
        connection.client(execute("")).await;
        connection.client(sync()).await;

        let errors = refused(&mut connection).await;
        assert_eq!(errors.len(), 1);
        assert!(errors[0].starts_with("COPY into encrypted column 'email'"));
    }

    /// PostgreSQL reads a file or program into the table as it reads copy data
    #[tokio::test]
    async fn a_copy_from_a_file_into_an_encrypted_column_is_refused() {
        let mut connection = Connection::encrypted().await;

        connection
            .client(query(
                "COPY public.users FROM '/tmp/users.csv' WITH (FORMAT csv)",
            ))
            .await;

        let errors = refused(&mut connection).await;
        assert!(errors[0].starts_with("COPY into encrypted column 'email'"));
    }

    #[tokio::test]
    async fn a_copy_into_only_plaintext_columns_of_an_encrypted_table_is_forwarded() {
        let mut connection = Connection::encrypted().await;
        let sql = "COPY users (id) FROM STDIN";

        connection.client(query(sql)).await;

        forwarded_unchanged(&mut connection, sql).await;
    }

    /// Proxy cannot tell whether a table it does not know has encrypted columns
    #[tokio::test]
    async fn a_copy_into_an_unknown_table_is_refused() {
        let mut connection = Connection::encrypted().await;

        connection.client(query("COPY unknown FROM STDIN")).await;

        let errors = refused(&mut connection).await;
        assert!(errors[0].starts_with("COPY into table 'unknown' is not supported"));
    }

    #[tokio::test]
    async fn a_copy_into_a_table_created_in_the_same_query_is_forwarded() {
        let mut connection = Connection::encrypted().await;
        let sql = "CREATE TABLE staging (id int); COPY staging FROM STDIN";

        connection.client(query(sql)).await;

        forwarded_unchanged(&mut connection, sql).await;
    }

    /// `CREATE TABLE ... AS` creates columns Proxy does not see
    #[tokio::test]
    async fn a_copy_with_no_column_list_into_a_table_with_no_known_columns_is_refused() {
        let mut connection = Connection::encrypted().await;

        connection
            .client(query(
                "CREATE TABLE staging AS SELECT * FROM users; COPY staging FROM STDIN",
            ))
            .await;

        let errors = refused(&mut connection).await;
        assert!(errors[0].starts_with("COPY into table 'staging' is not supported"));
    }

    /// `CREATE TABLE ... (LIKE ...)` copies columns Proxy does not see. The parser reads the
    /// `LIKE` clause as a column named `LIKE`.
    #[tokio::test]
    async fn a_copy_into_a_table_created_like_another_is_refused() {
        for sql in [
            "CREATE TABLE staging (LIKE users); COPY staging FROM STDIN",
            "CREATE TABLE staging (like users, note text); COPY staging FROM STDIN",
            "CREATE TABLE staging (LIKE users, note text); COPY staging (note) FROM STDIN",
        ] {
            let mut connection = Connection::encrypted().await;

            connection.client(query(sql)).await;

            let errors = refused(&mut connection).await;
            assert!(
                errors[0].starts_with("COPY into table 'staging' is not supported"),
                "{sql}"
            );
        }
    }

    /// PostgreSQL finds a table named without a schema in the temporary schema first, but a
    /// schema-qualified name is the table in that schema
    #[tokio::test]
    async fn a_schema_qualified_copy_is_not_hidden_by_a_table_of_the_same_name() {
        let mut connection = Connection::encrypted().await;

        connection
            .client(query(
                "CREATE TEMP TABLE users (id int, email text); COPY public.users FROM STDIN",
            ))
            .await;

        let errors = refused(&mut connection).await;
        assert!(errors[0].starts_with("COPY into encrypted column 'email' of table 'users'"));
    }

    #[tokio::test]
    async fn a_schema_qualified_copy_into_a_table_created_with_that_name_is_forwarded() {
        let mut connection = Connection::encrypted().await;
        let sql = "CREATE TABLE public.staging (id int); COPY public.staging FROM STDIN";

        connection.client(query(sql)).await;

        forwarded_unchanged(&mut connection, sql).await;
    }

    /// The Query is forwarded unchanged after a statement that fails type checking
    #[tokio::test]
    async fn a_copy_after_a_statement_that_fails_type_checking_is_still_checked() {
        let mut connection = Connection::encrypted().await;

        connection
            .client(query("SELECT * FROM unknown; COPY users FROM STDIN"))
            .await;

        let errors = refused(&mut connection).await;
        assert!(errors[0].starts_with("COPY into encrypted column 'email'"));
    }

    /// The query of a `COPY ... TO` is not type checked, so one that writes would send its
    /// values to PostgreSQL as plaintext
    #[tokio::test]
    async fn a_copy_of_a_query_that_writes_is_refused() {
        for sql in [
            "COPY (INSERT INTO users (email) VALUES ('alice@example.com') RETURNING id) TO STDOUT",
            "COPY (UPDATE users SET email = 'alice@example.com' RETURNING id) TO STDOUT",
            "COPY (DELETE FROM users WHERE email = 'alice@example.com' RETURNING id) TO STDOUT",
            "COPY (WITH i AS (INSERT INTO users (email) VALUES ('alice@example.com') RETURNING id) SELECT id FROM i) TO STDOUT",
            "COPY ((WITH i AS (INSERT INTO users (email) VALUES ('alice@example.com') RETURNING id) SELECT id FROM i)) TO STDOUT",
        ] {
            let mut connection = Connection::encrypted().await;

            connection.client(query(sql)).await;

            let errors = refused(&mut connection).await;
            assert!(
                errors[0].starts_with("COPY of a query that writes is not supported"),
                "{sql}: {errors:?}"
            );
        }
    }

    /// Proxy records a table created on this connection before PostgreSQL runs the `CREATE`, which
    /// may then not create it
    #[tokio::test]
    async fn a_copy_into_an_encrypted_table_is_not_hidden_by_a_table_created_on_the_connection() {
        let mut connection = Connection::encrypted().await;
        connection
            .client(query(
                "CREATE TABLE IF NOT EXISTS users (id int, email text); COPY users FROM STDIN",
            ))
            .await;
        let errors = refused(&mut connection).await;
        assert!(errors[0].starts_with("COPY into encrypted column 'email'"));

        let mut connection = Connection::encrypted().await;
        connection
            .client(query(
                "CREATE TEMP TABLE users (id int, email text); COPY unknown FROM STDIN",
            ))
            .await;
        refused(&mut connection).await;
        connection.client(query("COPY users FROM STDIN")).await;
        let errors = refused(&mut connection).await;
        assert!(errors[0].starts_with("COPY into encrypted column 'email'"));
    }

    /// PostgreSQL stops at the statement that fails, so it does not run the `CREATE` after it
    #[tokio::test]
    async fn a_table_created_after_a_statement_that_fails_type_checking_is_not_known() {
        let mut connection = Connection::encrypted().await;

        connection
            .client(query(
                "SELECT * FROM unknown; CREATE TABLE staging (id int); COPY staging FROM STDIN",
            ))
            .await;

        let errors = refused(&mut connection).await;
        assert!(errors[0].starts_with("COPY into table 'staging' is not supported"));
    }

    /// The loaded schema has tables by name, without their schema
    #[tokio::test]
    async fn a_copy_into_a_table_name_two_schemas_have_is_refused() {
        let mut config = TandemConfig::for_testing();
        config.encrypt.default_keyset_id = None;
        let encrypt_config = EncryptConfig::from_json(serde_json::json!({
            "v": 1,
            "tables": { "users": { "email": {} } }
        }));
        let mut plaintext = Table::new(Ident::new("users"));
        plaintext.add_column(Arc::new(eql_mapper::Column::native(Ident::with_quote(
            '"', "email",
        ))));
        let mut encrypted = Table::new(Ident::new("users"));
        encrypted.add_column(Arc::new(eql_mapper::Column::eql(
            Ident::with_quote('"', "email"),
            EqlTraits::all(),
        )));
        let mut schema = Schema::new("public");
        schema.add_table(plaintext);
        schema.add_table(encrypted);
        let mut connection = Connection::with_schema(config, encrypt_config, schema);
        connection.server(ready(b'I')).await;

        connection.client(query("COPY users FROM STDIN")).await;

        let errors = refused(&mut connection).await;
        assert!(errors[0].starts_with("COPY into table 'users' is not supported"));
    }

    #[tokio::test]
    async fn a_copy_into_an_unknown_column_of_a_known_table_is_refused() {
        let mut connection = Connection::encrypted().await;

        connection
            .client(query("COPY users (id, nickname) FROM STDIN"))
            .await;

        let errors = refused(&mut connection).await;
        assert!(errors[0].starts_with("COPY into table 'users' is not supported"));
    }

    /// Only an unquoted `LIKE` is the clause
    #[tokio::test]
    async fn a_copy_into_a_table_with_a_quoted_like_column_is_forwarded() {
        let mut connection = Connection::encrypted().await;
        let sql = r#"CREATE TABLE staging ("like" text); COPY staging FROM STDIN"#;

        connection.client(query(sql)).await;

        forwarded_unchanged(&mut connection, sql).await;
    }

    #[tokio::test]
    async fn a_copy_into_an_encrypted_column_is_forwarded_with_mapping_disabled() {
        for sql in [
            "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true; COPY users FROM STDIN",
            "SELECT * FROM unknown; SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true; COPY users FROM STDIN",
        ] {
            let mut connection = Connection::encrypted().await;

            connection.client(query(sql)).await;

            forwarded_unchanged(&mut connection, sql).await;
        }
    }

    /// Proxy returns the ciphertext, as it does not decrypt copy data
    #[tokio::test]
    async fn a_copy_of_an_encrypted_table_to_stdout_is_forwarded() {
        let mut connection = Connection::encrypted().await;
        let sql = "COPY users TO STDOUT";

        connection.client(query(sql)).await;

        forwarded_unchanged(&mut connection, sql).await;
    }

    /// Proxy returns the ciphertext, as it does not decrypt copy data
    #[tokio::test]
    async fn a_copy_to_stdout_of_a_query_that_only_reads_is_forwarded() {
        let mut connection = Connection::encrypted().await;
        let sql = "COPY (SELECT id, email FROM users) TO STDOUT";

        connection.client(query(sql)).await;

        forwarded_unchanged(&mut connection, sql).await;
    }

    #[tokio::test]
    async fn a_copy_of_a_query_that_writes_is_forwarded_with_no_encrypted_columns() {
        let mut connection = Connection::started().await;
        let sql =
            "COPY (INSERT INTO users (email) VALUES ('alice@example.com') RETURNING id) TO STDOUT";

        connection.client(query(sql)).await;

        forwarded_unchanged(&mut connection, sql).await;
    }
}
