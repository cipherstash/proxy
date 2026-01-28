use serde::Serialize;
use sqltk::parser::ast::Statement;

/// Statement type classification for metrics labels
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum StatementType {
    Insert,
    Update,
    Delete,
    Select,
    Other,
}

impl StatementType {
    /// Create from parsed AST statement
    pub fn from_statement(stmt: &Statement) -> Self {
        match stmt {
            Statement::Insert(_) => StatementType::Insert,
            Statement::Update { .. } => StatementType::Update,
            Statement::Delete(_) => StatementType::Delete,
            Statement::Query(_) => StatementType::Select,
            _ => StatementType::Other,
        }
    }

    /// Return lowercase label for metrics
    pub fn as_label(&self) -> &'static str {
        match self {
            StatementType::Insert => "insert",
            StatementType::Update => "update",
            StatementType::Delete => "delete",
            StatementType::Select => "select",
            StatementType::Other => "other",
        }
    }
}

/// Protocol type for metrics labels
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum ProtocolType {
    Simple,
    Extended,
}

impl ProtocolType {
    pub fn as_label(&self) -> &'static str {
        match self {
            ProtocolType::Simple => "simple",
            ProtocolType::Extended => "extended",
        }
    }
}

/// Metadata collected during statement processing for diagnostics
#[derive(Clone, Debug, Default)]
pub struct StatementMetadata {
    /// Type of SQL statement
    pub statement_type: Option<StatementType>,
    /// Protocol used (simple or extended)
    pub protocol: Option<ProtocolType>,
    /// Whether encryption/decryption was performed
    pub encrypted: bool,
    /// Number of encrypted values in the statement
    pub encrypted_values_count: usize,
    /// Approximate size of parameters in bytes
    pub param_bytes: usize,
    /// Query fingerprint (first 8 chars of normalized query hash)
    pub query_fingerprint: Option<String>,
    /// Whether the simple query contained multiple statements
    pub multi_statement: bool,
}

impl StatementMetadata {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_statement_type(mut self, stmt_type: StatementType) -> Self {
        self.statement_type = Some(stmt_type);
        self
    }

    pub fn with_protocol(mut self, protocol: ProtocolType) -> Self {
        self.protocol = Some(protocol);
        self
    }

    pub fn with_encrypted(mut self, encrypted: bool) -> Self {
        self.encrypted = encrypted;
        self
    }

    pub fn set_encrypted_values_count(&mut self, count: usize) {
        self.encrypted_values_count = count;
    }

    pub fn set_param_bytes(&mut self, bytes: usize) {
        self.param_bytes = bytes;
    }

    /// Set query fingerprint from SQL statement.
    ///
    /// Uses Blake3 keyed hashing with a per-instance random key to prevent dictionary attacks
    /// that could reveal SQL statements from fingerprints in logs/metrics.
    ///
    /// Fingerprints are instance-local identifiers for correlating log entries within a single
    /// proxy instance. They are NOT stable across restarts or deployments and should not
    /// be used for cross-instance correlation or persistent storage.
    pub fn set_query_fingerprint(&mut self, sql: &str) {
        use std::sync::LazyLock;

        // Random key generated once per proxy instance - makes fingerprints
        // resistant to dictionary attacks while remaining consistent within instance
        static FINGERPRINT_KEY: LazyLock<[u8; 32]> = LazyLock::new(rand::random);

        let hash = blake3::keyed_hash(&FINGERPRINT_KEY, sql.as_bytes());
        self.query_fingerprint = Some(hex::encode(&hash.as_bytes()[..4]));
    }

    pub fn set_multi_statement(&mut self, value: bool) {
        self.multi_statement = value;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqltk::parser::dialect::PostgreSqlDialect;
    use sqltk::parser::parser::Parser;

    fn parse(sql: &str) -> Statement {
        Parser::new(&PostgreSqlDialect {})
            .try_with_sql(sql)
            .unwrap()
            .parse_statement()
            .unwrap()
    }

    #[test]
    fn statement_type_from_statement() {
        assert_eq!(
            StatementType::from_statement(&parse("INSERT INTO foo VALUES (1)")),
            StatementType::Insert
        );
        assert_eq!(
            StatementType::from_statement(&parse("UPDATE foo SET bar = 1")),
            StatementType::Update
        );
        assert_eq!(
            StatementType::from_statement(&parse("DELETE FROM foo")),
            StatementType::Delete
        );
        assert_eq!(
            StatementType::from_statement(&parse("SELECT * FROM foo")),
            StatementType::Select
        );
        assert_eq!(
            StatementType::from_statement(&parse("CREATE TABLE foo (id INT)")),
            StatementType::Other
        );
    }

    #[test]
    fn statement_type_labels() {
        assert_eq!(StatementType::Insert.as_label(), "insert");
        assert_eq!(StatementType::Update.as_label(), "update");
        assert_eq!(StatementType::Delete.as_label(), "delete");
        assert_eq!(StatementType::Select.as_label(), "select");
        assert_eq!(StatementType::Other.as_label(), "other");
    }

    #[test]
    fn metadata_builder_pattern() {
        let metadata = StatementMetadata::new()
            .with_statement_type(StatementType::Insert)
            .with_protocol(ProtocolType::Extended)
            .with_encrypted(true);

        assert_eq!(metadata.statement_type, Some(StatementType::Insert));
        assert_eq!(metadata.protocol, Some(ProtocolType::Extended));
        assert!(metadata.encrypted);
    }

    #[test]
    fn query_fingerprint_is_deterministic() {
        let mut m1 = StatementMetadata::new();
        let mut m2 = StatementMetadata::new();

        m1.set_query_fingerprint("SELECT * FROM users WHERE id = $1");
        m2.set_query_fingerprint("SELECT * FROM users WHERE id = $1");

        assert_eq!(m1.query_fingerprint, m2.query_fingerprint);
    }

    #[test]
    fn multi_statement_flag_defaults_false() {
        let metadata = StatementMetadata::new();
        assert!(!metadata.multi_statement);
    }
}
