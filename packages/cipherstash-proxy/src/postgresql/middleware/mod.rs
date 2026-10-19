mod backend;
mod frontend;

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
            context::ProtocolStateSizes, test_operation_id, Column, Context, KeysetIdentifier,
            OperationId,
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
            let (reload_sender, _) = mpsc::unbounded_channel();
            let context = Context::new(
                1,
                Arc::new(TandemConfig::for_testing()),
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
            .get_statement_metrics_scope(&Bytes::new())
            .unwrap()
            .is_some());
        assert!(connection
            .context
            .get_statement_metrics_scope(&Bytes::from_static(b"a1"))
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
            .get_statement_metrics_scope(&Bytes::from_static(b"a1"))
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
            .get_statement_metrics_scope(&Bytes::from_static(b"a1"))
            .unwrap()
            .is_some());
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
}
