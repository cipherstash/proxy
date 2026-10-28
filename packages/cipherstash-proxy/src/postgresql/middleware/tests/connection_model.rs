//! Drives one connection with random client traffic against a model of the PostgreSQL state
//! that traffic produces, and checks that Proxy's mirror of that state stays in step.
//!
//! The model answers each message Proxy forwards as PostgreSQL would, so refusals Proxy turns
//! into failing messages fail here too. After every `ReadyForQuery` it checks that Proxy:
//! - holds no prepared statement PostgreSQL has released,
//! - holds no operation, portal or suspended execution once the connection is idle,
//! - agrees on `CIPHERSTASH.UNSAFE_DISABLE_MAPPING`, and
//! - agrees on whether the connection's temporary table exists.
//!
//! The model covers less than PostgreSQL does:
//! - each message gets its reply before the client sends the next, so pipelined batches
//!   are left to the harness tests,
//! - Execute always runs a portal to completion, and there is no Describe or Flush,
//! - each simple Query holds one statement,
//! - there are no savepoints, and
//! - only the statements a step marks as failing fail, besides the errors PostgreSQL
//!   raises for missing, duplicate or aborted-transaction messages.

use super::*;
use proptest::prelude::*;
use sqltk::parser::ast::{Ident, ObjectName, ObjectNamePart};
use std::collections::{HashMap, HashSet};

/// SQL sent with Parse
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExtendedSql {
    Select,
    SetMapping(bool),
    Deallocate(u8),
    DeallocateAll,
}

/// SQL sent with a simple Query
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SimpleSql {
    Select,
    SetMapping(bool),
    SetLocalMapping(bool),
    Begin,
    Commit,
    Rollback,
    DiscardAll,
    CreateTemporaryTable,
    DropTemporaryTable,
    Deallocate(u8),
}

/// What PostgreSQL runs for a statement or portal
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Sql {
    Extended(ExtendedSql),
    Simple(SimpleSql),
    /// A statement Proxy rewrote so that PostgreSQL fails it
    Refusal,
}

#[derive(Clone, Debug)]
enum Step {
    Parse { statement: u8, sql: ExtendedSql },
    Bind { statement: u8, portal: u8 },
    Execute { portal: u8, fails: bool },
    CloseStatement(u8),
    ClosePortal(u8),
    Sync,
    Query { sql: SimpleSql, fails: bool },
}

fn statement_name(n: u8) -> String {
    if n == 0 {
        String::new()
    } else {
        format!("s{n}")
    }
}

fn portal_name(n: u8) -> String {
    if n == 0 {
        String::new()
    } else {
        format!("p{n}")
    }
}

fn mapping(value: bool) -> &'static str {
    if value {
        "true"
    } else {
        "false"
    }
}

fn steps() -> impl Strategy<Value = Vec<Step>> {
    let extended_sql = prop_oneof![
        3 => Just(ExtendedSql::Select),
        2 => any::<bool>().prop_map(ExtendedSql::SetMapping),
        1 => (1_u8..3).prop_map(ExtendedSql::Deallocate),
        1 => Just(ExtendedSql::DeallocateAll),
    ];
    let simple_sql = prop_oneof![
        2 => Just(SimpleSql::Select),
        2 => any::<bool>().prop_map(SimpleSql::SetMapping),
        1 => any::<bool>().prop_map(SimpleSql::SetLocalMapping),
        2 => Just(SimpleSql::Begin),
        1 => Just(SimpleSql::Commit),
        1 => Just(SimpleSql::Rollback),
        1 => Just(SimpleSql::DiscardAll),
        1 => Just(SimpleSql::CreateTemporaryTable),
        1 => Just(SimpleSql::DropTemporaryTable),
        1 => (1_u8..3).prop_map(SimpleSql::Deallocate),
    ];
    let step = prop_oneof![
        4 => (0_u8..3, extended_sql).prop_map(|(statement, sql)| Step::Parse { statement, sql }),
        4 => (0_u8..3, 0_u8..2).prop_map(|(statement, portal)| Step::Bind { statement, portal }),
        4 => (0_u8..2, prop::bool::weighted(0.1))
            .prop_map(|(portal, fails)| Step::Execute { portal, fails }),
        1 => (0_u8..3).prop_map(Step::CloseStatement),
        1 => (0_u8..2).prop_map(Step::ClosePortal),
        3 => Just(Step::Sync),
        3 => (simple_sql, prop::bool::weighted(0.1))
            .prop_map(|(sql, fails)| Step::Query { sql, fails }),
    ];
    prop::collection::vec(step, 1..60)
}

/// The settings and temporary table a transaction rollback restores
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Restorable {
    mapping_disabled: bool,
    local_mapping_disabled: Option<bool>,
    temporary_table: bool,
}

impl Restorable {
    /// A plain `SET` inside a transaction also replaces any `SET LOCAL` value
    fn set_mapping(&mut self, value: bool) {
        self.mapping_disabled = value;
        self.local_mapping_disabled = None;
    }

    fn effective_mapping_disabled(&self) -> bool {
        self.local_mapping_disabled.unwrap_or(self.mapping_disabled)
    }
}

/// The state of the PostgreSQL connection behind Proxy
#[derive(Debug, Default)]
struct Postgres {
    statements: HashMap<String, Sql>,
    /// Portals, and whether each has run to completion
    portals: HashMap<String, (Sql, bool)>,
    current: Restorable,
    /// Restored when an explicit transaction rolls back
    transaction_start: Option<Restorable>,
    /// Whether the explicit transaction has failed
    transaction_failed: bool,
    /// Restored when the implicit transaction of an extended-protocol batch fails
    batch_start: Option<Restorable>,
    /// Whether PostgreSQL ignores extended-protocol messages until Sync
    skipping: bool,
}

/// PostgreSQL's reply to one message
enum Reply {
    Message(BackendMessage),
    Error,
}

impl Postgres {
    fn in_transaction(&self) -> bool {
        self.transaction_start.is_some()
    }

    fn status(&self) -> TransactionStatus {
        match (self.in_transaction(), self.transaction_failed) {
            (false, _) => TransactionStatus::Idle,
            (true, false) => TransactionStatus::InTransaction,
            (true, true) => TransactionStatus::FailedTransaction,
        }
    }

    fn begin_batch(&mut self) {
        if !self.in_transaction() && self.batch_start.is_none() {
            self.batch_start = Some(self.current);
        }
    }

    /// An error inside a transaction or extended-protocol batch
    fn fail(&mut self) {
        if self.in_transaction() {
            self.transaction_failed = true;
        } else if let Some(start) = self.batch_start.take() {
            self.current = start;
        }
    }

    fn end_implicit_transaction(&mut self) {
        if !self.in_transaction() {
            self.batch_start = None;
            self.current.local_mapping_disabled = None;
            self.portals.clear();
        }
    }

    fn end_explicit_transaction(&mut self, commit: bool) {
        if let Some(start) = self.transaction_start.take() {
            if !commit {
                self.current = start;
            }
        }
        self.transaction_failed = false;
        self.current.local_mapping_disabled = None;
        self.portals.clear();
    }

    fn discard_all(&mut self) {
        self.statements.retain(|name, _| name.is_empty());
        self.portals.clear();
        self.current = Restorable::default();
    }

    fn deallocate(&mut self, name: u8) -> Reply {
        if self.statements.remove(&statement_name(name)).is_some() {
            command(b"DEALLOCATE")
        } else {
            Reply::Error
        }
    }

    fn parse(&mut self, name: String, sql: Sql) -> Option<Reply> {
        if self.skipping {
            return None;
        }
        self.begin_batch();
        // PostgreSQL releases the unnamed statement before it parses the new one
        if name.is_empty() {
            self.statements.remove(&name);
        }
        let reply = if self.transaction_failed
            || sql == Sql::Refusal
            || (!name.is_empty() && self.statements.contains_key(&name))
        {
            Reply::Error
        } else {
            self.statements.insert(name, sql);
            Reply::Message(BackendMessage::ParseComplete)
        };
        Some(self.extended(reply))
    }

    fn bind(&mut self, statement: &[u8], portal: String) -> Option<Reply> {
        if self.skipping {
            return None;
        }
        self.begin_batch();
        let statement = String::from_utf8_lossy(statement).into_owned();
        let reply = match self.statements.get(&statement) {
            Some(sql)
                if !self.transaction_failed
                    && (portal.is_empty() || !self.portals.contains_key(&portal)) =>
            {
                self.portals.insert(portal, (*sql, false));
                Reply::Message(BackendMessage::BindComplete)
            }
            _ => Reply::Error,
        };
        Some(self.extended(reply))
    }

    fn execute(&mut self, portal: &[u8], fails: bool) -> Option<Reply> {
        if self.skipping {
            return None;
        }
        self.begin_batch();
        let portal = String::from_utf8_lossy(portal).into_owned();
        let reply = match self.portals.get_mut(&portal) {
            Some((Sql::Extended(ExtendedSql::Select), true)) if !self.transaction_failed => {
                // A SELECT portal run to completion stays runnable and returns no more rows
                command(b"SELECT 0")
            }
            Some((sql, done)) if !self.transaction_failed && !*done => {
                // Any other portal is done once it has run, and running it again fails
                *done = true;
                let sql = *sql;
                match sql {
                    Sql::Extended(ExtendedSql::Select | ExtendedSql::SetMapping(_)) if fails => {
                        Reply::Error
                    }
                    Sql::Extended(ExtendedSql::Select) => command(b"SELECT 1"),
                    Sql::Extended(ExtendedSql::SetMapping(value)) => {
                        self.current.set_mapping(value);
                        command(b"SET")
                    }
                    Sql::Extended(ExtendedSql::Deallocate(name)) => self.deallocate(name),
                    Sql::Extended(ExtendedSql::DeallocateAll) => {
                        self.statements.retain(|name, _| name.is_empty());
                        command(b"DEALLOCATE ALL")
                    }
                    Sql::Simple(_) | Sql::Refusal => Reply::Error,
                }
            }
            _ => Reply::Error,
        };
        Some(self.extended(reply))
    }

    /// An extended-protocol error makes PostgreSQL ignore messages until Sync
    fn extended(&mut self, reply: Reply) -> Reply {
        if matches!(reply, Reply::Error) {
            self.fail();
            self.skipping = true;
        }
        reply
    }

    fn sync(&mut self) -> BackendMessage {
        self.skipping = false;
        self.end_implicit_transaction();
        BackendMessage::ReadyForQuery(self.status())
    }

    fn query(&mut self, sql: Sql, fails: bool) -> Vec<BackendMessage> {
        // A simple Query runs in the unnamed statement and portal, releasing the client's
        self.statements.remove("");
        self.portals.remove("");
        let Sql::Simple(sql) = sql else {
            self.fail();
            return self.query_end(Reply::Error);
        };
        let exits_transaction = matches!(sql, SimpleSql::Commit | SimpleSql::Rollback);
        if self.transaction_failed && !exits_transaction {
            return self.query_end(Reply::Error);
        }
        let reply = match sql {
            SimpleSql::Select
            | SimpleSql::SetMapping(_)
            | SimpleSql::SetLocalMapping(_)
            | SimpleSql::CreateTemporaryTable
            | SimpleSql::DropTemporaryTable
                if fails =>
            {
                self.fail();
                Reply::Error
            }
            SimpleSql::Select => command(b"SELECT 1"),
            SimpleSql::SetMapping(value) => {
                self.current.set_mapping(value);
                command(b"SET")
            }
            SimpleSql::SetLocalMapping(value) => {
                if self.in_transaction() {
                    self.current.local_mapping_disabled = Some(value);
                }
                command(b"SET")
            }
            SimpleSql::Begin => {
                if !self.in_transaction() {
                    self.transaction_start = Some(self.current);
                }
                command(b"BEGIN")
            }
            SimpleSql::Commit => {
                let failed = self.transaction_failed;
                self.end_explicit_transaction(!failed);
                command(if failed { b"ROLLBACK" } else { b"COMMIT" })
            }
            SimpleSql::Rollback => {
                self.end_explicit_transaction(false);
                command(b"ROLLBACK")
            }
            SimpleSql::DiscardAll if self.in_transaction() => {
                self.fail();
                Reply::Error
            }
            SimpleSql::DiscardAll => {
                self.discard_all();
                command(b"DISCARD ALL")
            }
            SimpleSql::CreateTemporaryTable if !self.current.temporary_table => {
                self.current.temporary_table = true;
                command(b"CREATE TABLE")
            }
            SimpleSql::DropTemporaryTable if self.current.temporary_table => {
                self.current.temporary_table = false;
                command(b"DROP TABLE")
            }
            SimpleSql::Deallocate(name) => match self.deallocate(name) {
                Reply::Error => {
                    self.fail();
                    Reply::Error
                }
                reply => reply,
            },
            SimpleSql::CreateTemporaryTable | SimpleSql::DropTemporaryTable => {
                self.fail();
                Reply::Error
            }
        };
        self.query_end(reply)
    }

    fn query_end(&mut self, reply: Reply) -> Vec<BackendMessage> {
        if matches!(reply, Reply::Error) && self.in_transaction() {
            self.transaction_failed = true;
        }
        self.end_implicit_transaction();
        vec![
            match reply {
                Reply::Message(message) => message,
                Reply::Error => error(),
            },
            BackendMessage::ReadyForQuery(self.status()),
        ]
    }
}

fn command(tag: &'static [u8]) -> Reply {
    Reply::Message(command_complete(tag))
}

/// Runs `steps` against a connection and the model, checking the connection after each
/// readiness
struct Run {
    connection: Connection,
    postgres: Postgres,
    /// SQL text Proxy may forward, and what PostgreSQL runs for it
    sql: HashMap<String, Sql>,
    batch_open: bool,
}

impl Run {
    fn new(connection: Connection) -> Self {
        let mut sql = HashMap::new();
        let select = connection.select.to_owned();
        sql.insert(select.clone(), Sql::Extended(ExtendedSql::Select));
        for value in [true, false] {
            sql.insert(
                format!(
                    "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = {}",
                    mapping(value)
                ),
                Sql::Extended(ExtendedSql::SetMapping(value)),
            );
            sql.insert(
                format!(
                    "SET LOCAL CIPHERSTASH.UNSAFE_DISABLE_MAPPING = {}",
                    mapping(value)
                ),
                Sql::Simple(SimpleSql::SetLocalMapping(value)),
            );
        }
        for name in 1..3 {
            sql.insert(
                format!("DEALLOCATE {}", statement_name(name)),
                Sql::Extended(ExtendedSql::Deallocate(name)),
            );
        }
        sql.insert(
            "DEALLOCATE ALL".to_owned(),
            Sql::Extended(ExtendedSql::DeallocateAll),
        );
        for (text, simple) in [
            ("BEGIN", SimpleSql::Begin),
            ("COMMIT", SimpleSql::Commit),
            ("ROLLBACK", SimpleSql::Rollback),
            ("DISCARD ALL", SimpleSql::DiscardAll),
            (
                "CREATE TEMPORARY TABLE scratch (id int)",
                SimpleSql::CreateTemporaryTable,
            ),
            ("DROP TABLE scratch", SimpleSql::DropTemporaryTable),
        ] {
            sql.insert(text.to_owned(), Sql::Simple(simple));
        }
        Self {
            connection,
            postgres: Postgres::default(),
            sql,
            batch_open: false,
        }
    }

    fn extended_text(&self, sql: ExtendedSql) -> String {
        match sql {
            ExtendedSql::Select => self.connection.select.to_owned(),
            ExtendedSql::SetMapping(value) => {
                format!(
                    "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = {}",
                    mapping(value)
                )
            }
            ExtendedSql::Deallocate(name) => format!("DEALLOCATE {}", statement_name(name)),
            ExtendedSql::DeallocateAll => "DEALLOCATE ALL".to_owned(),
        }
    }

    fn simple_text(&self, sql: SimpleSql) -> String {
        match sql {
            SimpleSql::Select => self.connection.select.to_owned(),
            SimpleSql::SetMapping(value) => self.extended_text(ExtendedSql::SetMapping(value)),
            SimpleSql::SetLocalMapping(value) => format!(
                "SET LOCAL CIPHERSTASH.UNSAFE_DISABLE_MAPPING = {}",
                mapping(value)
            ),
            SimpleSql::Begin => "BEGIN".to_owned(),
            SimpleSql::Commit => "COMMIT".to_owned(),
            SimpleSql::Rollback => "ROLLBACK".to_owned(),
            SimpleSql::DiscardAll => "DISCARD ALL".to_owned(),
            SimpleSql::CreateTemporaryTable => "CREATE TEMPORARY TABLE scratch (id int)".to_owned(),
            SimpleSql::DropTemporaryTable => "DROP TABLE scratch".to_owned(),
            SimpleSql::Deallocate(name) => self.extended_text(ExtendedSql::Deallocate(name)),
        }
    }

    /// What PostgreSQL runs for SQL Proxy forwarded
    fn classify(&self, text: &[u8], simple: bool) -> Sql {
        let text = String::from_utf8_lossy(text);
        match self.sql.get(text.as_ref()) {
            Some(Sql::Extended(ExtendedSql::Select)) if simple => Sql::Simple(SimpleSql::Select),
            Some(Sql::Extended(ExtendedSql::SetMapping(value))) if simple => {
                Sql::Simple(SimpleSql::SetMapping(*value))
            }
            Some(Sql::Extended(ExtendedSql::Deallocate(name))) if simple => {
                Sql::Simple(SimpleSql::Deallocate(*name))
            }
            Some(sql) => *sql,
            None if text.contains("__cipherstash_") || text.starts_with("DO ") => Sql::Refusal,
            // Proxy rewrote the statement that reads the encrypted column
            None if simple => Sql::Simple(SimpleSql::Select),
            None => Sql::Extended(ExtendedSql::Select),
        }
    }

    /// Sends `message`; returns the message Proxy forwarded, if any
    async fn send(&mut self, message: FrontendMessage) -> (OperationId, Option<FrontendMessage>) {
        let (operation, output) = self.connection.client_output(message).await;
        let forwarded = match output {
            FrontendMiddlewareOutput::Forward(message)
            | FrontendMiddlewareOutput::ForwardThenFlush(message) => Some(message),
            FrontendMiddlewareOutput::Suppress(_) => None,
            output => panic!("unexpected frontend output {output:?}"),
        };
        (operation, forwarded)
    }

    async fn reply(&mut self, operation: OperationId, reply: Option<Reply>) {
        match reply {
            Some(Reply::Message(message)) => {
                self.connection.server(operation, message).await;
            }
            Some(Reply::Error) => {
                self.connection.server(operation, error()).await;
            }
            None => {}
        }
    }

    async fn step(&mut self, step: Step) {
        match step {
            Step::Query { .. } if self.batch_open => {
                Box::pin(self.step(Step::Sync)).await;
                Box::pin(self.step(step)).await;
            }
            Step::Parse { statement, sql } => {
                self.batch_open = true;
                let message = FrontendMessage::Parse(Parse {
                    statement: Bytes::from(statement_name(statement)),
                    query: Bytes::from(self.extended_text(sql)),
                    parameter_types: vec![],
                });
                let (operation, forwarded) = self.send(message).await;
                let reply = match forwarded {
                    Some(FrontendMessage::Parse(parse)) => {
                        let sql = self.classify(&parse.query, false);
                        let name = String::from_utf8_lossy(&parse.statement).into_owned();
                        self.postgres.parse(name, sql)
                    }
                    None => None,
                    Some(other) => panic!("Parse forwarded as {other:?}"),
                };
                self.reply(operation, reply).await;
            }
            Step::Bind { statement, portal } => {
                self.batch_open = true;
                let message = bind_message_to(statement_name(statement), portal_name(portal));
                let (operation, forwarded) = self.send(message).await;
                let reply = match forwarded {
                    Some(FrontendMessage::Bind(bind)) => {
                        let portal = String::from_utf8_lossy(&bind.portal).into_owned();
                        self.postgres.bind(&bind.statement, portal)
                    }
                    None => None,
                    Some(other) => panic!("Bind forwarded as {other:?}"),
                };
                self.reply(operation, reply).await;
            }
            Step::Execute { portal, fails } => {
                self.batch_open = true;
                let (operation, forwarded) = self
                    .send(FrontendMessage::Execute(Execute {
                        portal: Bytes::from(portal_name(portal)),
                        max_rows: 0,
                    }))
                    .await;
                let reply = match forwarded {
                    Some(FrontendMessage::Execute(execute)) => {
                        self.postgres.execute(&execute.portal, fails)
                    }
                    None => None,
                    Some(other) => panic!("Execute forwarded as {other:?}"),
                };
                self.reply(operation, reply).await;
            }
            Step::CloseStatement(name) | Step::ClosePortal(name) => {
                self.batch_open = true;
                let (target, name) = match step {
                    Step::CloseStatement(_) => (DescribeTarget::Statement, statement_name(name)),
                    _ => (DescribeTarget::Portal, portal_name(name)),
                };
                let (operation, forwarded) = self
                    .send(FrontendMessage::Close(Close {
                        target,
                        name: Bytes::from(name.clone()),
                    }))
                    .await;
                let reply = forwarded.and_then(|_| {
                    if self.postgres.skipping {
                        return None;
                    }
                    match target {
                        DescribeTarget::Statement => self.postgres.statements.remove(&name),
                        DescribeTarget::Portal => {
                            self.postgres.portals.remove(&name).map(|(sql, _)| sql)
                        }
                    };
                    Some(Reply::Message(BackendMessage::CloseComplete))
                });
                self.reply(operation, reply).await;
            }
            Step::Sync => {
                self.batch_open = false;
                let (operation, forwarded) = self.send(FrontendMessage::Sync).await;
                assert!(matches!(forwarded, Some(FrontendMessage::Sync)));
                let ready = self.postgres.sync();
                self.connection.server(operation, ready).await;
                self.check();
            }
            Step::Query { sql, fails } => {
                let text = self.simple_text(sql);
                let (operation, forwarded) =
                    self.send(FrontendMessage::Query(Bytes::from(text))).await;
                let Some(FrontendMessage::Query(query)) = forwarded else {
                    panic!("Query forwarded as {forwarded:?}");
                };
                let sql = self.classify(&query, true);
                for message in self.postgres.query(sql, fails) {
                    self.connection.server(operation, message).await;
                }
                self.check();
            }
        }
    }

    /// Proxy's mirror of the connection agrees with PostgreSQL
    fn check(&self) {
        let postgres = &self.postgres;
        let held: HashSet<String> = statement_names(&self.connection).into_iter().collect();
        for name in &held {
            assert!(
                postgres.statements.contains_key(name),
                "Proxy holds statement {name:?}, which PostgreSQL released"
            );
        }
        if postgres.status() == TransactionStatus::Idle {
            let sizes = self.connection.sizes();
            assert_eq!(sizes.operations, 0, "operations held at idle: {sizes:?}");
            assert_eq!(sizes.portals, 0, "portals held at idle: {sizes:?}");
            assert_eq!(
                sizes.portal_operations, 0,
                "portal operations held at idle: {sizes:?}"
            );
            assert_eq!(
                sizes.suspended_executions, 0,
                "suspended executions held at idle: {sizes:?}"
            );
        }
        assert_eq!(
            self.connection.context.unsafe_disable_mapping().unwrap(),
            postgres.current.effective_mapping_disabled(),
            "UNSAFE_DISABLE_MAPPING"
        );
        let scratch = ObjectName(vec![ObjectNamePart::Identifier(Ident::new("scratch"))]);
        assert_eq!(
            self.connection
                .context
                .get_table_resolver()
                .resolve_table(&scratch)
                .is_ok(),
            postgres.current.temporary_table,
            "temporary table"
        );
    }
}

fn bind_message_to(statement: String, portal: String) -> FrontendMessage {
    FrontendMessage::Bind(Bind {
        portal: Bytes::from(portal),
        statement: Bytes::from(statement),
        parameter_formats: vec![],
        parameters: vec![],
        result_formats: vec![],
    })
}

fn run(connect: fn() -> Connection, steps: Vec<Step>) {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
        .block_on(async {
            let mut run = Run::new(connect());
            for step in steps {
                run.step(step).await;
            }
            run.step(Step::Sync).await;
        });
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(256))]

    #[test]
    fn plaintext_connection_mirrors_postgresql(steps in steps()) {
        run(Connection::new, steps);
    }

    #[test]
    fn encrypted_connection_mirrors_postgresql(steps in steps()) {
        run(Connection::encrypted, steps);
    }
}
