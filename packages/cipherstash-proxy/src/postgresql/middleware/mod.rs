mod backend;
mod frontend;
#[cfg(test)]
mod settings_rollback_tests;

use super::Context;
use crate::{error::Error, proxy::EncryptionService};
use backend::Backend;
use frontend::Frontend;
use pg_proto::{
    BackendMessage, BackendMiddlewareOutput, FrontendMessage, FrontendMiddlewareOutput,
    IntermediaryMiddleware, IntermediaryMiddlewareFactory, OperationId,
};

pub struct CipherStashMiddleware<S: EncryptionService + Clone> {
    frontend: Frontend<S>,
    backend: Backend<S>,
}

#[derive(Clone)]
pub struct CipherStashMiddlewareFactory<S: EncryptionService + Clone>(pub Context<S>);

impl<S, ServerContext, ClientContext> IntermediaryMiddlewareFactory<ServerContext, ClientContext>
    for CipherStashMiddlewareFactory<S>
where
    S: EncryptionService + Clone,
    ServerContext: Sync,
    ClientContext: Sync,
{
    type Handler = CipherStashMiddleware<S>;
    fn create(&self, _: &ServerContext, _: &ClientContext) -> Self::Handler {
        CipherStashMiddleware::new(self.0.clone())
    }
}

impl<S: EncryptionService + Clone> CipherStashMiddleware<S> {
    pub fn new(context: Context<S>) -> Self {
        Self {
            frontend: Frontend::new(context.clone()),
            backend: Backend::new(context),
        }
    }
}

impl<S, ServerContext, ClientContext> IntermediaryMiddleware<(), ServerContext, ClientContext>
    for CipherStashMiddleware<S>
where
    S: EncryptionService + Clone,
    ServerContext: Sync,
    ClientContext: Sync,
{
    type Error = Error;

    async fn frontend_operation(
        &mut self,
        _: &ServerContext,
        _: &ClientContext,
        _: &mut (),
        operation: OperationId,
        message: FrontendMessage,
    ) -> Result<FrontendMiddlewareOutput, Error> {
        self.frontend.intercept(operation.into(), message).await
    }

    async fn backend_operation(
        &mut self,
        _: &ServerContext,
        _: &ClientContext,
        _: &mut (),
        operation: Option<OperationId>,
        message: BackendMessage,
    ) -> Result<BackendMiddlewareOutput, Error> {
        self.backend
            .intercept(operation.map(Into::into), message)
            .await
    }
}

/// Connection protocol state must stay bounded however long a client connection lives.
///
/// Each test drives the frontend and backend middleware with the messages a client and
/// PostgreSQL exchange for one pattern, many times over on one connection.
#[cfg(test)]
mod tests {
    use super::{Backend, Frontend};
    use crate::{
        config::TandemConfig,
        error::Error,
        postgresql::{
            context::{ProtocolStateSizes, Statement},
            test_operation_id, Column, Context, KeysetIdentifier, OperationId,
        },
        proxy::{EncryptConfig, EncryptionService},
    };
    use bytes::Bytes;
    use eql_mapper::Schema;
    use pg_proto::{
        BackendMessage, Bind, Close, Describe, DescribeTarget, Execute, FrontendMessage, Parse,
        TransactionStatus,
    };
    use std::sync::Arc;
    use tokio::sync::mpsc;

    const ITERATIONS: usize = 200;

    #[derive(Clone)]
    struct TestService;

    #[async_trait::async_trait]
    impl EncryptionService for TestService {
        async fn encrypt(
            &self,
            _keyset_id: Option<KeysetIdentifier>,
            _plaintexts: Vec<Option<cipherstash_client::encryption::Plaintext>>,
            _columns: &[Option<Column>],
        ) -> Result<Vec<Option<crate::EqlOutput>>, Error> {
            Ok(vec![])
        }

        async fn decrypt(
            &self,
            _keyset_id: Option<KeysetIdentifier>,
            _ciphertexts: Vec<Option<crate::EqlCiphertext>>,
        ) -> Result<Vec<Option<cipherstash_client::encryption::Plaintext>>, Error> {
            Ok(vec![])
        }

        async fn decrypt_inbound_eql(
            &self,
            _keyset_id: Option<KeysetIdentifier>,
            _ciphertexts: Vec<Option<crate::EqlCiphertext>>,
        ) -> Result<Vec<Option<cipherstash_client::encryption::Plaintext>>, Error> {
            Ok(vec![])
        }
    }

    struct Connection {
        frontend: Frontend<TestService>,
        backend: Backend<TestService>,
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
                Arc::new(rustls::RootCertStore::empty()),
                TestService,
                reload_sender,
            );
            Self {
                frontend: Frontend::new(context.clone()),
                backend: Backend::new(context.clone()),
                context,
            }
        }

        /// Client sends a message; returns the operation pg-proto assigns it
        async fn client(&mut self, message: FrontendMessage) -> OperationId {
            let operation = test_operation_id();
            self.frontend.intercept(operation, message).await.unwrap();
            operation
        }

        async fn server(&mut self, operation: OperationId, message: BackendMessage) {
            self.backend
                .intercept(Some(operation), message)
                .await
                .unwrap();
        }

        async fn parse(&mut self, statement: &'static [u8]) -> OperationId {
            self.client(FrontendMessage::Parse(Parse {
                statement: Bytes::from_static(statement),
                query: Bytes::from_static(b"select 1"),
                parameter_types: vec![],
            }))
            .await
        }

        async fn parse_named(&mut self, statement: String) -> OperationId {
            self.client(FrontendMessage::Parse(Parse {
                statement: Bytes::from(statement),
                query: Bytes::from_static(b"select 1"),
                parameter_types: vec![],
            }))
            .await
        }

        async fn parse_sql(&mut self, statement: &'static [u8], sql: String) -> OperationId {
            self.client(FrontendMessage::Parse(Parse {
                statement: Bytes::from_static(statement),
                query: Bytes::from(sql),
                parameter_types: vec![],
            }))
            .await
        }

        /// Parse, Bind, Execute and Sync `sql` as the unnamed statement, with PostgreSQL's
        /// responses
        async fn extended(&mut self, sql: String, tag: &'static [u8]) {
            let parse = self.parse_sql(b"", sql).await;
            let bind = self.bind(Bytes::new(), Bytes::new()).await;
            let execute = self.execute(Bytes::new()).await;
            let sync = self.client(FrontendMessage::Sync).await;
            self.server(parse, BackendMessage::ParseComplete).await;
            self.server(bind, BackendMessage::BindComplete).await;
            self.server(execute, command_complete(tag)).await;
            self.server(sync, ready(TransactionStatus::Idle)).await;
        }

        async fn bind(&mut self, statement: Bytes, portal: Bytes) -> OperationId {
            self.client(FrontendMessage::Bind(Bind {
                portal,
                statement,
                parameter_formats: vec![],
                parameters: vec![],
                result_formats: vec![],
            }))
            .await
        }

        async fn execute(&mut self, portal: Bytes) -> OperationId {
            self.client(FrontendMessage::Execute(Execute {
                portal,
                max_rows: 0,
            }))
            .await
        }

        async fn query(&mut self, sql: &str) -> OperationId {
            self.client(FrontendMessage::Query(Bytes::from(sql.to_owned())))
                .await
        }

        fn sizes(&self) -> ProtocolStateSizes {
            self.context.protocol_state_sizes()
        }
    }

    fn command_complete(tag: &'static [u8]) -> BackendMessage {
        BackendMessage::CommandComplete(Bytes::from_static(tag))
    }

    fn ready(status: TransactionStatus) -> BackendMessage {
        BackendMessage::ReadyForQuery(status)
    }

    fn error() -> BackendMessage {
        BackendMessage::ErrorResponse(crate::postgresql::diagnostics::invalid_sql_statement(
            "error".to_owned(),
        ))
    }

    #[tokio::test]
    async fn prepare_and_describe_without_execute_retains_nothing() {
        let mut connection = Connection::new();
        for _ in 0..ITERATIONS {
            let parse = connection.parse(b"").await;
            let describe = connection
                .client(FrontendMessage::Describe(Describe {
                    target: DescribeTarget::Statement,
                    name: Bytes::new(),
                }))
                .await;
            let sync = connection.client(FrontendMessage::Sync).await;

            connection
                .server(parse, BackendMessage::ParseComplete)
                .await;
            connection.server(describe, BackendMessage::NoData).await;
            connection
                .server(sync, ready(TransactionStatus::Idle))
                .await;
        }
        // Only the unnamed statement's own state remains
        assert!(connection.sizes().statement_metrics <= 1);
        assert_eq!(connection.sizes().operations, 0);
    }

    #[tokio::test]
    async fn executing_a_prepared_statement_retains_nothing() {
        let mut connection = Connection::new();
        let parse = connection.parse(b"S_1").await;
        let sync = connection.client(FrontendMessage::Sync).await;
        connection
            .server(parse, BackendMessage::ParseComplete)
            .await;
        connection
            .server(sync, ready(TransactionStatus::Idle))
            .await;
        let prepared = connection.sizes();

        for _ in 0..ITERATIONS {
            let bind = connection
                .bind(Bytes::from_static(b"S_1"), Bytes::new())
                .await;
            let execute = connection.execute(Bytes::new()).await;
            let sync = connection.client(FrontendMessage::Sync).await;
            connection.server(bind, BackendMessage::BindComplete).await;
            connection
                .server(execute, command_complete(b"SELECT 1"))
                .await;
            connection
                .server(sync, ready(TransactionStatus::Idle))
                .await;
        }
        // Only the prepared statement's own state remains
        assert_eq!(connection.sizes(), prepared);
    }

    #[tokio::test]
    async fn pipelined_batch_failing_part_way_retains_nothing() {
        let mut connection = Connection::new();
        for _ in 0..ITERATIONS {
            let mut operations = vec![];
            for _ in 0..2 {
                let parse = connection.parse(b"").await;
                let bind = connection.bind(Bytes::new(), Bytes::new()).await;
                let execute = connection.execute(Bytes::new()).await;
                operations.push((parse, bind, execute));
            }
            let sync = connection.client(FrontendMessage::Sync).await;

            // The first Execute fails; PostgreSQL skips the rest until Sync
            let (parse, bind, execute) = operations[0];
            connection
                .server(parse, BackendMessage::ParseComplete)
                .await;
            connection.server(bind, BackendMessage::BindComplete).await;
            connection.server(execute, error()).await;
            connection
                .server(sync, ready(TransactionStatus::Idle))
                .await;
        }
        let sizes = connection.sizes();
        assert_eq!(sizes.operations, 0);
        assert_eq!(sizes.portals, 0);
        assert_eq!(sizes.portal_operations, 0);
        assert!(sizes.statement_metrics <= 1);
    }

    #[tokio::test]
    async fn unique_named_portals_retain_nothing() {
        let mut connection = Connection::new();
        let parse = connection.parse(b"S_1").await;
        let sync = connection.client(FrontendMessage::Sync).await;
        connection
            .server(parse, BackendMessage::ParseComplete)
            .await;
        connection
            .server(sync, ready(TransactionStatus::InTransaction))
            .await;

        for i in 0..ITERATIONS {
            let portal = Bytes::from(format!("C_{i}"));
            let bind = connection
                .bind(Bytes::from_static(b"S_1"), portal.clone())
                .await;
            let execute = connection.execute(portal.clone()).await;
            let close = connection
                .client(FrontendMessage::Close(Close {
                    target: DescribeTarget::Portal,
                    name: portal,
                }))
                .await;
            let sync = connection.client(FrontendMessage::Sync).await;

            connection.server(bind, BackendMessage::BindComplete).await;
            connection
                .server(execute, command_complete(b"SELECT 1"))
                .await;
            connection
                .server(close, BackendMessage::CloseComplete)
                .await;
            connection
                .server(sync, ready(TransactionStatus::InTransaction))
                .await;
        }
        let sizes = connection.sizes();
        assert_eq!(sizes.operations, 0);
        assert_eq!(sizes.portals, 0);
        assert_eq!(sizes.portal_operations, 0);
    }

    #[tokio::test]
    async fn sql_deallocate_retains_nothing() {
        let mut connection = Connection::new();
        for i in 0..ITERATIONS {
            let name = format!("_pg3_{i}");
            let parse = connection.parse_named(name.clone()).await;
            let bind = connection
                .bind(Bytes::from(name.clone()), Bytes::new())
                .await;
            let execute = connection.execute(Bytes::new()).await;
            let sync = connection.client(FrontendMessage::Sync).await;
            connection
                .server(parse, BackendMessage::ParseComplete)
                .await;
            connection.server(bind, BackendMessage::BindComplete).await;
            connection
                .server(execute, command_complete(b"SELECT 1"))
                .await;
            connection
                .server(sync, ready(TransactionStatus::Idle))
                .await;

            let query = connection.query(&format!("DEALLOCATE {name}")).await;
            connection
                .server(query, command_complete(b"DEALLOCATE"))
                .await;
            connection
                .server(query, ready(TransactionStatus::Idle))
                .await;
        }
        assert_eq!(connection.sizes(), ProtocolStateSizes::default());
    }

    #[tokio::test]
    async fn sql_deallocate_all_and_discard_all_retain_nothing() {
        for (sql, tag) in [
            ("DEALLOCATE ALL", &b"DEALLOCATE ALL"[..]),
            ("DISCARD ALL", &b"DISCARD ALL"[..]),
        ] {
            let mut connection = Connection::new();
            for i in 0..ITERATIONS {
                let parse = connection.parse_named(format!("a{i}")).await;
                let sync = connection.client(FrontendMessage::Sync).await;
                connection
                    .server(parse, BackendMessage::ParseComplete)
                    .await;
                connection
                    .server(sync, ready(TransactionStatus::Idle))
                    .await;
            }

            let query = connection.query(sql).await;
            connection
                .server(
                    query,
                    BackendMessage::CommandComplete(Bytes::from_static(tag)),
                )
                .await;
            connection
                .server(query, ready(TransactionStatus::Idle))
                .await;

            assert_eq!(connection.sizes(), ProtocolStateSizes::default(), "{sql}");
        }
    }

    /// Parse and Sync each named statement, with PostgreSQL's responses
    async fn prepare(connection: &mut Connection, names: &[&str]) {
        for name in names {
            let parse = connection.parse_named((*name).to_owned()).await;
            let sync = connection.client(FrontendMessage::Sync).await;
            connection
                .server(parse, BackendMessage::ParseComplete)
                .await;
            connection
                .server(sync, ready(TransactionStatus::Idle))
                .await;
        }
    }

    fn statement_names(connection: &Connection) -> Vec<String> {
        let mut names = connection.context.statement_operation_names();
        names.sort();
        names
    }

    #[tokio::test]
    async fn multi_statement_query_deallocates_each_statement() {
        let mut connection = Connection::new();
        prepare(&mut connection, &["a1", "a2", "a3"]).await;

        let query = connection.query("DEALLOCATE a1; DEALLOCATE a2").await;
        connection
            .server(query, command_complete(b"DEALLOCATE"))
            .await;
        connection
            .server(query, command_complete(b"DEALLOCATE"))
            .await;
        connection
            .server(query, ready(TransactionStatus::Idle))
            .await;

        assert_eq!(statement_names(&connection), vec!["a3"]);
        assert_eq!(connection.sizes().operations, 0);
    }

    #[tokio::test]
    async fn query_failing_part_way_keeps_the_statements_it_did_not_deallocate() {
        let mut connection = Connection::new();
        prepare(&mut connection, &["a1", "a2"]).await;

        let query = connection
            .query("DEALLOCATE a1; SELECT 1/0; DEALLOCATE a2")
            .await;
        connection
            .server(query, command_complete(b"DEALLOCATE"))
            .await;
        connection.server(query, error()).await;
        connection
            .server(query, ready(TransactionStatus::Idle))
            .await;

        assert_eq!(statement_names(&connection), vec!["a2"]);
        assert_eq!(connection.sizes().operations, 0);
    }

    #[tokio::test]
    async fn deallocate_folds_unquoted_names_and_keeps_quoted_ones() {
        let mut connection = Connection::new();
        prepare(&mut connection, &["Mixed", "upper", "UPPER"]).await;

        let query = connection
            .query(r#"DEALLOCATE "Mixed"; DEALLOCATE UPPER"#)
            .await;
        connection
            .server(query, command_complete(b"DEALLOCATE"))
            .await;
        connection
            .server(query, command_complete(b"DEALLOCATE"))
            .await;
        connection
            .server(query, ready(TransactionStatus::Idle))
            .await;

        assert_eq!(statement_names(&connection), vec!["UPPER"]);
    }

    #[tokio::test]
    async fn deallocate_prepare_removes_the_statement() {
        let mut connection = Connection::new();
        prepare(&mut connection, &["a1", "a2"]).await;

        let query = connection.query("DEALLOCATE PREPARE a1").await;
        connection
            .server(query, command_complete(b"DEALLOCATE"))
            .await;
        connection
            .server(query, ready(TransactionStatus::Idle))
            .await;

        assert_eq!(statement_names(&connection), vec!["a2"]);
    }

    #[tokio::test]
    async fn deallocation_that_does_not_match_the_command_tag_removes_nothing() {
        let mut connection = Connection::new();
        prepare(&mut connection, &["a1", "a2"]).await;

        // Proxy queued DEALLOCATE a1 and DEALLOCATE a2, but PostgreSQL reports DEALLOCATE ALL
        let query = connection.query("DEALLOCATE a1; DEALLOCATE a2").await;
        connection
            .server(query, command_complete(b"DEALLOCATE ALL"))
            .await;
        connection
            .server(query, command_complete(b"DEALLOCATE"))
            .await;
        connection
            .server(query, ready(TransactionStatus::Idle))
            .await;

        // Out of step with PostgreSQL, so nothing is applied: keeping statements is safe
        assert_eq!(statement_names(&connection), vec!["a1", "a2"]);
        assert_eq!(connection.sizes().operations, 0);
    }

    #[tokio::test]
    async fn sql_deallocate_all_keeps_a_statement_with_no_recorded_parse() {
        let mut connection = Connection::new();
        // An encrypted statement whose Parse operation was never recorded
        connection
            .context
            .test_add_statement(
                Bytes::from_static(b"s1"),
                Statement::new(vec![], vec![], vec![], vec![], vec![]),
            )
            .unwrap();

        let query = connection.query("DEALLOCATE ALL").await;
        connection
            .server(query, command_complete(b"DEALLOCATE ALL"))
            .await;
        connection
            .server(query, ready(TransactionStatus::Idle))
            .await;

        // Keeping it can only leak; removing it would let its next Bind through unencrypted
        assert_eq!(connection.sizes().statements, 1);
    }

    #[tokio::test]
    async fn extended_protocol_sql_deallocate_retains_nothing() {
        let mut connection = Connection::new();
        let mut after_first = None;
        for i in 0..ITERATIONS {
            let name = format!("_pg3_{i}");
            let parse = connection.parse_named(name.clone()).await;
            let sync = connection.client(FrontendMessage::Sync).await;
            connection
                .server(parse, BackendMessage::ParseComplete)
                .await;
            connection
                .server(sync, ready(TransactionStatus::Idle))
                .await;

            connection
                .extended(format!("DEALLOCATE {name}"), b"DEALLOCATE")
                .await;
            after_first.get_or_insert_with(|| connection.sizes());
        }
        // Only the unnamed statement holding the last DEALLOCATE remains
        assert_eq!(connection.sizes(), after_first.unwrap());
        assert_eq!(connection.sizes().statement_operations, 1);
    }

    #[tokio::test]
    async fn extended_protocol_deallocate_all_and_discard_all_retain_nothing() {
        for (sql, tag) in [
            ("DEALLOCATE ALL", &b"DEALLOCATE ALL"[..]),
            ("DISCARD ALL", &b"DISCARD ALL"[..]),
        ] {
            let mut connection = Connection::new();
            for i in 0..ITERATIONS {
                let parse = connection.parse_named(format!("a{i}")).await;
                let sync = connection.client(FrontendMessage::Sync).await;
                connection
                    .server(parse, BackendMessage::ParseComplete)
                    .await;
                connection
                    .server(sync, ready(TransactionStatus::Idle))
                    .await;
            }

            let tag = Bytes::copy_from_slice(tag);
            let parse = connection.parse_sql(b"", sql.to_owned()).await;
            let bind = connection.bind(Bytes::new(), Bytes::new()).await;
            let execute = connection.execute(Bytes::new()).await;
            let sync = connection.client(FrontendMessage::Sync).await;
            connection
                .server(parse, BackendMessage::ParseComplete)
                .await;
            connection.server(bind, BackendMessage::BindComplete).await;
            connection
                .server(execute, BackendMessage::CommandComplete(tag))
                .await;
            connection
                .server(sync, ready(TransactionStatus::Idle))
                .await;

            // Only the unnamed statement that ran the deallocation remains
            assert_eq!(connection.sizes().statement_operations, 1, "{sql}");
            assert_eq!(connection.sizes().statement_metrics_scopes, 1, "{sql}");
            assert_eq!(connection.sizes().portals, 0, "{sql}");
        }
    }

    #[tokio::test]
    async fn sql_deallocate_all_keeps_the_unnamed_statement() {
        let mut connection = Connection::new();
        let unnamed = connection.parse(b"").await;
        let named = connection.parse(b"a1").await;
        let sync = connection.client(FrontendMessage::Sync).await;
        connection
            .server(unnamed, BackendMessage::ParseComplete)
            .await;
        connection
            .server(named, BackendMessage::ParseComplete)
            .await;
        connection
            .server(sync, ready(TransactionStatus::Idle))
            .await;

        let query = connection.query("DEALLOCATE ALL").await;
        connection
            .server(query, command_complete(b"DEALLOCATE ALL"))
            .await;
        connection
            .server(query, ready(TransactionStatus::Idle))
            .await;

        assert!(connection
            .context
            .test_statement_metrics_scope(&Bytes::new())
            .unwrap()
            .is_some());
        assert!(connection
            .context
            .test_statement_metrics_scope(&Bytes::from_static(b"a1"))
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn failed_sql_deallocate_keeps_the_statement() {
        let mut connection = Connection::new();
        let parse = connection.parse(b"a1").await;
        let sync = connection.client(FrontendMessage::Sync).await;
        connection
            .server(parse, BackendMessage::ParseComplete)
            .await;
        connection
            .server(sync, ready(TransactionStatus::InTransaction))
            .await;

        // DEALLOCATE inside a failed transaction
        let query = connection.query("DEALLOCATE a1").await;
        connection.server(query, error()).await;
        connection
            .server(query, ready(TransactionStatus::FailedTransaction))
            .await;

        assert!(connection
            .context
            .test_statement_metrics_scope(&Bytes::from_static(b"a1"))
            .unwrap()
            .is_some());
        assert_eq!(connection.sizes().operations, 0);
    }

    #[tokio::test]
    async fn sql_deallocate_keeps_a_statement_prepared_after_it() {
        let mut connection = Connection::new();
        let parse = connection.parse(b"a1").await;
        let sync = connection.client(FrontendMessage::Sync).await;
        connection
            .server(parse, BackendMessage::ParseComplete)
            .await;
        connection
            .server(sync, ready(TransactionStatus::Idle))
            .await;

        let query = connection.query("DEALLOCATE a1").await;
        // The client prepares the name again before PostgreSQL confirms the DEALLOCATE
        let parse = connection.parse(b"a1").await;
        let sync = connection.client(FrontendMessage::Sync).await;

        connection
            .server(query, command_complete(b"DEALLOCATE"))
            .await;
        connection
            .server(query, ready(TransactionStatus::Idle))
            .await;
        connection
            .server(parse, BackendMessage::ParseComplete)
            .await;
        connection
            .server(sync, ready(TransactionStatus::Idle))
            .await;

        assert!(connection
            .context
            .test_statement_metrics_scope(&Bytes::from_static(b"a1"))
            .unwrap()
            .is_some());
    }

    #[tokio::test]
    async fn failed_parse_after_sql_deallocate_does_not_restore_the_statement() {
        let mut connection = Connection::new();
        let parse = connection.parse(b"a1").await;
        let sync = connection.client(FrontendMessage::Sync).await;
        connection
            .server(parse, BackendMessage::ParseComplete)
            .await;
        connection
            .server(sync, ready(TransactionStatus::Idle))
            .await;

        let query = connection.query("DEALLOCATE a1").await;
        // The client prepares the name again before PostgreSQL confirms the DEALLOCATE,
        // and PostgreSQL rejects that Parse
        let parse = connection.parse(b"a1").await;
        let sync = connection.client(FrontendMessage::Sync).await;

        connection
            .server(query, command_complete(b"DEALLOCATE"))
            .await;
        connection
            .server(query, ready(TransactionStatus::Idle))
            .await;
        connection.server(parse, error()).await;
        connection
            .server(sync, ready(TransactionStatus::Idle))
            .await;

        // PostgreSQL has no statement `a1`, so Proxy must not restore the deallocated one
        assert!(connection
            .context
            .test_statement_metrics_scope(&Bytes::from_static(b"a1"))
            .unwrap()
            .is_none());
        assert_eq!(connection.sizes().settings_statements, 0);
        assert_eq!(connection.sizes().statement_operations, 0);
    }

    #[tokio::test]
    async fn failed_close_keeps_the_statement_releasable_by_sql_deallocate() {
        let mut connection = Connection::new();
        prepare(&mut connection, &["s1"]).await;

        // An earlier message in the batch fails, so PostgreSQL skips the Close and keeps `s1`
        let parse = connection.parse(b"z").await;
        connection
            .client(FrontendMessage::Close(Close {
                target: DescribeTarget::Statement,
                name: Bytes::from_static(b"s1"),
            }))
            .await;
        let sync = connection.client(FrontendMessage::Sync).await;
        connection.server(parse, error()).await;
        connection
            .server(sync, ready(TransactionStatus::Idle))
            .await;

        let s1 = Bytes::from_static(b"s1");
        assert!(connection
            .context
            .test_statement_metrics_scope(&s1)
            .unwrap()
            .is_some());

        let query = connection.query("DEALLOCATE s1").await;
        connection
            .server(query, command_complete(b"DEALLOCATE"))
            .await;
        connection
            .server(query, ready(TransactionStatus::Idle))
            .await;
        // Released with the statement, not leaked for the life of the connection
        assert!(connection
            .context
            .test_statement_metrics_scope(&s1)
            .unwrap()
            .is_none());
        assert_eq!(statement_names(&connection), Vec::<String>::new());
    }

    #[tokio::test]
    async fn failed_reparse_keeps_the_deallocation_of_the_retained_statement() {
        let mut connection = Connection::new();
        prepare(&mut connection, &["x"]).await;
        let parse = connection.parse_sql(b"d", "DEALLOCATE x".to_owned()).await;
        let sync = connection.client(FrontendMessage::Sync).await;
        connection
            .server(parse, BackendMessage::ParseComplete)
            .await;
        connection
            .server(sync, ready(TransactionStatus::Idle))
            .await;

        // PostgreSQL rejects a second Parse of `d` and keeps the first
        let parse = connection.parse_sql(b"d", "SELECT 1".to_owned()).await;
        let sync = connection.client(FrontendMessage::Sync).await;
        connection.server(parse, error()).await;
        connection
            .server(sync, ready(TransactionStatus::Idle))
            .await;

        let bind = connection
            .bind(Bytes::from_static(b"d"), Bytes::new())
            .await;
        let execute = connection.execute(Bytes::new()).await;
        let sync = connection.client(FrontendMessage::Sync).await;
        connection.server(bind, BackendMessage::BindComplete).await;
        connection
            .server(execute, command_complete(b"DEALLOCATE"))
            .await;
        connection
            .server(sync, ready(TransactionStatus::Idle))
            .await;
        assert_eq!(statement_names(&connection), vec!["d".to_owned()]);
    }

    #[test]
    fn deallocation_command_tags() {
        use super::backend::is_deallocation;
        assert!(is_deallocation(b"DEALLOCATE"));
        assert!(is_deallocation(b"DEALLOCATE ALL"));
        assert!(is_deallocation(b"DISCARD ALL"));
        assert!(!is_deallocation(b"DISCARD PLANS"));
        assert!(!is_deallocation(b"SELECT 1"));
    }

    /// `DISCARD ALL` and `RESET` return the connection's Proxy settings to their startup
    /// values once PostgreSQL confirms them, so a pooled connection does not carry one
    /// client's keyset or disabled mapping over to the next client.
    mod settings_reset {
        use super::*;

        const KEYSET_ID: &str =
            "SET CIPHERSTASH.KEYSET_ID = '7e5ea5a6-4e1c-4f4d-8a9c-9f0b4f6a0c11'";

        /// A connection to a Proxy with no default keyset, so a keyset can be set per connection
        fn connection() -> Connection {
            let mut config = TandemConfig::for_testing();
            config.encrypt.default_keyset_id = None;
            Connection::with_config(config)
        }

        impl Connection {
            /// Send a simple Query and reply with PostgreSQL's `CommandComplete` for it
            async fn simple(&mut self, sql: &str, tag: &'static [u8]) {
                let query = self.query(sql).await;
                self.server(query, command_complete(tag)).await;
                self.server(query, ready(TransactionStatus::Idle)).await;
            }

            /// Select a keyset, then disable mapping
            async fn set_keyset_and_disable_mapping(&mut self) {
                self.simple(KEYSET_ID, b"SET").await;
                self.simple("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", b"SET")
                    .await;
                assert!(self.context.keyset_identifier().unwrap().is_some());
                assert!(self.context.unsafe_disable_mapping().unwrap());
            }

            /// Bind and execute the named statement `name` through the unnamed portal
            async fn execute_prepared(&mut self, name: &'static [u8], tag: &'static [u8]) {
                let bind = self.bind(Bytes::from_static(name), Bytes::new()).await;
                let execute = self.execute(Bytes::new()).await;
                let sync = self.client(FrontendMessage::Sync).await;
                self.server(bind, BackendMessage::BindComplete).await;
                self.server(execute, command_complete(tag)).await;
                self.server(sync, ready(TransactionStatus::Idle)).await;
            }

            /// Prepare `sql` as the named statement `name`
            async fn prepare(&mut self, name: &'static [u8], sql: &str) {
                let parse = self.parse_sql(name, sql.to_owned()).await;
                let sync = self.client(FrontendMessage::Sync).await;
                self.server(parse, BackendMessage::ParseComplete).await;
                self.server(sync, ready(TransactionStatus::Idle)).await;
            }

            fn assert_startup_settings(&self) {
                assert_eq!(self.context.keyset_identifier().unwrap(), None);
                assert!(!self.context.unsafe_disable_mapping().unwrap());
            }
        }

        #[tokio::test]
        async fn a_prepared_keyset_set_applies_again_when_executed_after_a_reset() {
            let mut connection = connection();
            connection.prepare(b"set_keyset", KEYSET_ID).await;
            connection.execute_prepared(b"set_keyset", b"SET").await;
            connection.simple("RESET ALL", b"RESET").await;
            assert_eq!(connection.context.keyset_identifier().unwrap(), None);

            connection.execute_prepared(b"set_keyset", b"SET").await;
            assert!(connection.context.keyset_identifier().unwrap().is_some());
        }

        #[tokio::test]
        async fn a_prepared_mapping_set_applies_again_when_executed_after_a_reset() {
            let mut connection = connection();
            connection
                .prepare(b"disable", "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true")
                .await;
            connection.execute_prepared(b"disable", b"SET").await;
            connection.simple("DISCARD ALL", b"DISCARD ALL").await;
            assert!(!connection.context.unsafe_disable_mapping().unwrap());

            connection
                .prepare(b"disable", "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true")
                .await;
            connection.simple("RESET ALL", b"RESET").await;
            connection.execute_prepared(b"disable", b"SET").await;
            assert!(connection.context.unsafe_disable_mapping().unwrap());
        }

        #[tokio::test]
        async fn a_prepared_keyset_set_executed_after_a_pending_reset_is_kept() {
            let mut connection = connection();
            connection.simple(KEYSET_ID, b"SET").await;
            connection.prepare(b"set_keyset", KEYSET_ID).await;

            // PostgreSQL runs the reset, then the prepared SET
            let reset = connection.query("RESET ALL").await;
            let bind = connection
                .bind(Bytes::from_static(b"set_keyset"), Bytes::new())
                .await;
            let execute = connection.execute(Bytes::new()).await;
            let sync = connection.client(FrontendMessage::Sync).await;
            connection.server(reset, command_complete(b"RESET")).await;
            connection
                .server(reset, ready(TransactionStatus::Idle))
                .await;
            connection.server(bind, BackendMessage::BindComplete).await;
            connection.server(execute, command_complete(b"SET")).await;
            connection
                .server(sync, ready(TransactionStatus::Idle))
                .await;

            assert!(connection.context.keyset_identifier().unwrap().is_some());
        }

        #[tokio::test]
        async fn discard_all_in_a_simple_query() {
            let mut connection = connection();
            connection.set_keyset_and_disable_mapping().await;
            connection.simple("DISCARD ALL", b"DISCARD ALL").await;
            connection.assert_startup_settings();
        }

        #[tokio::test]
        async fn reset_all_in_a_simple_query() {
            let mut connection = connection();
            connection.set_keyset_and_disable_mapping().await;
            connection.simple("RESET ALL", b"RESET").await;
            connection.assert_startup_settings();
        }

        #[tokio::test]
        async fn discard_all_in_the_extended_protocol() {
            let mut connection = connection();
            connection.set_keyset_and_disable_mapping().await;
            connection
                .extended("DISCARD ALL".to_owned(), b"DISCARD ALL")
                .await;
            connection.assert_startup_settings();
        }

        #[tokio::test]
        async fn reset_all_in_the_extended_protocol() {
            let mut connection = connection();
            connection.set_keyset_and_disable_mapping().await;
            connection.extended("RESET ALL".to_owned(), b"RESET").await;
            connection.assert_startup_settings();
        }

        #[tokio::test]
        async fn discard_all_falls_back_to_the_default_keyset() {
            let mut connection = Connection::new();
            connection
                .simple("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", b"SET")
                .await;
            connection.simple("DISCARD ALL", b"DISCARD ALL").await;
            connection.assert_startup_settings();
            assert!(connection.context.default_keyset_id().is_some());
        }

        #[tokio::test]
        async fn reset_of_each_setting_resets_only_that_setting() {
            let mut connection = connection();
            connection.set_keyset_and_disable_mapping().await;

            // A PostgreSQL setting
            connection.simple("RESET search_path", b"RESET").await;
            assert!(connection.context.keyset_identifier().unwrap().is_some());
            assert!(connection.context.unsafe_disable_mapping().unwrap());

            connection
                .simple("RESET CIPHERSTASH.UNSAFE_DISABLE_MAPPING", b"RESET")
                .await;
            assert!(connection.context.keyset_identifier().unwrap().is_some());
            assert!(!connection.context.unsafe_disable_mapping().unwrap());

            connection
                .simple("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true", b"SET")
                .await;
            connection
                .extended("RESET cipherstash.keyset_id".to_owned(), b"RESET")
                .await;
            assert_eq!(connection.context.keyset_identifier().unwrap(), None);
            assert!(connection.context.unsafe_disable_mapping().unwrap());
        }

        #[tokio::test]
        async fn reset_of_either_keyset_setting_clears_the_keyset() {
            // Proxy keeps one keyset, so resetting either setting fails closed rather than
            // keeping a keyset chosen by the other one
            for reset in [
                "RESET CIPHERSTASH.KEYSET_ID",
                "RESET CIPHERSTASH.KEYSET_NAME",
            ] {
                let mut connection = connection();
                connection
                    .simple("SET CIPHERSTASH.KEYSET_NAME = 'tenant'", b"SET")
                    .await;
                connection.simple(reset, b"RESET").await;
                assert_eq!(
                    connection.context.keyset_identifier().unwrap(),
                    None,
                    "{reset}"
                );
            }
        }

        #[tokio::test]
        async fn a_quoted_name_including_the_period_names_the_setting() {
            let mut connection = connection();
            connection.set_keyset_and_disable_mapping().await;

            connection
                .simple(r#"RESET "cipherstash.keyset_id""#, b"RESET")
                .await;
            assert_eq!(connection.context.keyset_identifier().unwrap(), None);

            connection
                .simple(
                    r#"SET "CIPHERSTASH.UNSAFE_DISABLE_MAPPING" TO DEFAULT"#,
                    b"SET",
                )
                .await;
            assert!(!connection.context.unsafe_disable_mapping().unwrap());
        }

        #[tokio::test]
        async fn settings_are_kept_until_postgresql_confirms_the_reset() {
            let mut connection = connection();
            connection.set_keyset_and_disable_mapping().await;

            let query = connection.query("DISCARD ALL").await;
            assert!(connection.context.keyset_identifier().unwrap().is_some());
            assert!(connection.context.unsafe_disable_mapping().unwrap());

            connection
                .server(query, command_complete(b"DISCARD ALL"))
                .await;
            connection.assert_startup_settings();
        }

        #[tokio::test]
        async fn a_failed_reset_keeps_the_settings() {
            let mut connection = connection();
            connection.set_keyset_and_disable_mapping().await;

            // Inside an aborted transaction PostgreSQL rejects each statement
            for sql in ["DISCARD ALL", "RESET ALL", "RESET ALL; RESET ALL"] {
                let query = connection.query(sql).await;
                connection.server(query, error()).await;
                connection
                    .server(query, ready(TransactionStatus::FailedTransaction))
                    .await;
            }

            let parse = connection.parse_sql(b"", "RESET ALL".to_owned()).await;
            let bind = connection.bind(Bytes::new(), Bytes::new()).await;
            let execute = connection.execute(Bytes::new()).await;
            let sync = connection.client(FrontendMessage::Sync).await;
            connection
                .server(parse, BackendMessage::ParseComplete)
                .await;
            connection.server(bind, BackendMessage::BindComplete).await;
            connection.server(execute, error()).await;
            connection
                .server(sync, ready(TransactionStatus::FailedTransaction))
                .await;

            assert!(connection.context.keyset_identifier().unwrap().is_some());
            assert!(connection.context.unsafe_disable_mapping().unwrap());
            assert_eq!(connection.sizes().operations, 0);
        }

        #[tokio::test]
        async fn a_set_sent_after_a_reset_is_kept() {
            let mut connection = connection();
            connection.simple(KEYSET_ID, b"SET").await;

            // The next client's SET is sent before PostgreSQL confirms the reset
            let reset = connection.query("DISCARD ALL").await;
            let set = connection.query(KEYSET_ID).await;
            connection
                .server(reset, command_complete(b"DISCARD ALL"))
                .await;
            connection
                .server(reset, ready(TransactionStatus::Idle))
                .await;
            connection.server(set, command_complete(b"SET")).await;
            connection.server(set, ready(TransactionStatus::Idle)).await;

            assert!(connection.context.keyset_identifier().unwrap().is_some());
        }

        #[tokio::test]
        async fn a_set_of_one_setting_after_a_reset_keeps_the_other_reset() {
            let mut connection = connection();
            connection.set_keyset_and_disable_mapping().await;

            let parse = connection.parse_sql(b"", "RESET ALL".to_owned()).await;
            let bind = connection.bind(Bytes::new(), Bytes::new()).await;
            let execute = connection.execute(Bytes::new()).await;
            let sync = connection.client(FrontendMessage::Sync).await;
            let set = connection
                .query("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true")
                .await;
            connection
                .server(parse, BackendMessage::ParseComplete)
                .await;
            connection.server(bind, BackendMessage::BindComplete).await;
            connection.server(execute, command_complete(b"RESET")).await;
            connection
                .server(sync, ready(TransactionStatus::Idle))
                .await;
            connection.server(set, command_complete(b"SET")).await;
            connection.server(set, ready(TransactionStatus::Idle)).await;

            assert_eq!(connection.context.keyset_identifier().unwrap(), None);
            assert!(connection.context.unsafe_disable_mapping().unwrap());
        }

        #[tokio::test]
        async fn a_failed_query_rolls_back_its_earlier_resets() {
            let mut connection = connection();
            connection.set_keyset_and_disable_mapping().await;

            // PostgreSQL runs the Query as one implicit transaction, so the second RESET's
            // error also rolls back the first
            let query = connection
                .query("RESET CIPHERSTASH.UNSAFE_DISABLE_MAPPING; RESET ALL")
                .await;
            connection.server(query, command_complete(b"RESET")).await;
            connection.server(query, error()).await;
            connection
                .server(query, ready(TransactionStatus::Idle))
                .await;

            assert!(connection.context.keyset_identifier().unwrap().is_some());
            assert!(connection.context.unsafe_disable_mapping().unwrap());
        }

        #[tokio::test]
        async fn prepared_reset_runs_each_time_it_is_executed() {
            let mut connection = connection();
            let parse = connection.parse_sql(b"reset", "RESET ALL".to_owned()).await;
            let sync = connection.client(FrontendMessage::Sync).await;
            connection
                .server(parse, BackendMessage::ParseComplete)
                .await;
            connection
                .server(sync, ready(TransactionStatus::Idle))
                .await;

            for _ in 0..2 {
                connection.set_keyset_and_disable_mapping().await;
                let bind = connection
                    .bind(Bytes::from_static(b"reset"), Bytes::new())
                    .await;
                let execute = connection.execute(Bytes::new()).await;
                let sync = connection.client(FrontendMessage::Sync).await;
                connection.server(bind, BackendMessage::BindComplete).await;
                connection.server(execute, command_complete(b"RESET")).await;
                connection
                    .server(sync, ready(TransactionStatus::Idle))
                    .await;
                connection.assert_startup_settings();
            }
        }

        #[tokio::test]
        async fn closing_a_prepared_reset_retains_nothing() {
            let mut connection = connection();
            for i in 0..ITERATIONS {
                let name = Bytes::from(format!("reset_{i}"));
                let parse = connection
                    .client(FrontendMessage::Parse(Parse {
                        statement: name.clone(),
                        query: Bytes::from_static(b"RESET ALL"),
                        parameter_types: vec![],
                    }))
                    .await;
                let close = connection
                    .client(FrontendMessage::Close(Close {
                        target: DescribeTarget::Statement,
                        name,
                    }))
                    .await;
                let sync = connection.client(FrontendMessage::Sync).await;
                connection
                    .server(parse, BackendMessage::ParseComplete)
                    .await;
                connection
                    .server(close, BackendMessage::CloseComplete)
                    .await;
                connection
                    .server(sync, ready(TransactionStatus::Idle))
                    .await;
            }
        }

        #[tokio::test]
        async fn set_to_default_resets_that_setting() {
            let mut connection = connection();
            connection.set_keyset_and_disable_mapping().await;

            connection
                .simple("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING TO DEFAULT", b"SET")
                .await;
            assert!(connection.context.keyset_identifier().unwrap().is_some());
            assert!(!connection.context.unsafe_disable_mapping().unwrap());

            connection
                .extended("SET CIPHERSTASH.KEYSET_ID = default".to_owned(), b"SET")
                .await;
            assert_eq!(connection.context.keyset_identifier().unwrap(), None);
        }

        #[tokio::test]
        async fn set_keyset_name_to_default_is_not_a_keyset_named_default() {
            let mut connection = connection();
            connection
                .simple("SET CIPHERSTASH.KEYSET_NAME = 'tenant'", b"SET")
                .await;
            connection
                .simple("SET CIPHERSTASH.KEYSET_NAME TO DEFAULT", b"SET")
                .await;
            assert_eq!(connection.context.keyset_identifier().unwrap(), None);

            // Not a request for a keyset, so not refused when there is a default keyset
            let mut connection = Connection::new();
            let message = FrontendMessage::Query(Bytes::from_static(
                b"SET CIPHERSTASH.KEYSET_NAME TO DEFAULT",
            ));
            let output = connection
                .frontend
                .intercept(test_operation_id(), message.clone())
                .await
                .unwrap();
            assert!(
                matches!(output, pg_proto::FrontendMiddlewareOutput::Forward(forwarded) if forwarded == message)
            );
        }

        #[tokio::test]
        async fn a_failed_query_rolls_back_its_earlier_set_to_default() {
            let mut connection = connection();
            connection.set_keyset_and_disable_mapping().await;

            // The last SET fails, and PostgreSQL rolls back the implicit transaction, so the
            // earlier SET TO DEFAULT is undone with it
            let query = connection
                .query("SET search_path = public; SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING TO DEFAULT; SET CIPHERSTASH.KEYSET_ID TO DEFAULT")
                .await;
            connection.server(query, command_complete(b"SET")).await;
            connection.server(query, command_complete(b"SET")).await;
            connection.server(query, error()).await;
            connection
                .server(query, ready(TransactionStatus::Idle))
                .await;

            assert!(connection.context.keyset_identifier().unwrap().is_some());
            assert!(connection.context.unsafe_disable_mapping().unwrap());
        }
    }
}
