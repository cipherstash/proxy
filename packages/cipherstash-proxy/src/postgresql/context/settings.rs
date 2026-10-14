//! Values set on one connection with Proxy's `SET CIPHERSTASH.*` control API.
//!
//! The values follow PostgreSQL's `SET` scoping, so that a connection pooler in
//! front of Proxy can rely on the same rules it relies on for PostgreSQL:
//!
//! - `SET` / `SET SESSION` survives commit; rollback restores its prior value.
//! - `SET LOCAL` lasts until the end of the current transaction. Outside an
//!   explicit transaction block that is the end of the statement's implicit
//!   transaction, as in PostgreSQL.
//! - Savepoints restore both scopes; releasing a savepoint keeps its changes.
//! - `DISCARD ALL` resets every value. Poolers send it before handing a server
//!   connection to a different client (PgBouncer's default `server_reset_query`).

use super::KeysetIdentifier;
use crate::error::{EncryptError, Error};
use crate::proxy::schema::postgres_identifier;
use cipherstash_client::IdentifiedBy;
use sqltk::parser::ast::ContextModifier;
use sqltk::parser::ast::{DiscardObject, Expr, Set, Statement, Value};

/// A validated change, recorded during preparation and applied after execution.
#[derive(Clone, Debug)]
pub enum SettingChange {
    Keyset(SettingScope, KeysetIdentifier),
    Mapping(SettingScope, bool),
    DiscardAll,
    ResetAll,
    ResetKeyset(SettingScope),
    ResetMapping(SettingScope),
}

impl SettingChange {
    pub fn parse(statement: &Statement) -> Result<Option<Self>, Error> {
        if matches!(
            statement,
            Statement::Discard {
                object_type: DiscardObject::ALL
            }
        ) {
            return Ok(Some(Self::DiscardAll));
        }
        let Statement::Set(Set::SingleAssignment {
            scope,
            variable,
            values,
            ..
        }) = statement
        else {
            return Ok(None);
        };
        let variable = variable.to_string().to_ascii_uppercase();
        // RESET is normalized to SET ... TO DEFAULT by SqlParser. DEFAULT
        // follows the same scope as a value, including a temporary local reset.
        let scope = SettingScope::from(*scope);
        let is_default = matches!(values.as_slice(), [Expr::Identifier(value)]
            if value.quote_style.is_none() && value.value.eq_ignore_ascii_case("DEFAULT"));
        if is_default {
            return Ok(match variable.as_str() {
                "ALL" if scope == SettingScope::Session => Some(Self::ResetAll),
                "CIPHERSTASH.KEYSET_ID" | "CIPHERSTASH.KEYSET_NAME" => {
                    Some(Self::ResetKeyset(scope))
                }
                "CIPHERSTASH.UNSAFE_DISABLE_MAPPING" => Some(Self::ResetMapping(scope)),
                _ => None,
            });
        }
        if variable == "CIPHERSTASH.UNSAFE_DISABLE_MAPPING" {
            return Ok(match values.first() {
                Some(Expr::Value(value)) if matches!(value.value, Value::Boolean(_)) => {
                    let Value::Boolean(value) = value.value else {
                        unreachable!()
                    };
                    Some(Self::Mapping(scope, value))
                }
                _ => None,
            });
        }
        if variable != "CIPHERSTASH.KEYSET_ID" && variable != "CIPHERSTASH.KEYSET_NAME" {
            return Ok(None);
        }
        let value = match values.first() {
            Some(Expr::Value(value)) => match &value.value {
                Value::SingleQuotedString(value) | Value::DoubleQuotedString(value) => {
                    Some(value.clone())
                }
                Value::Number(value, _) => Some(value.to_string()),
                _ => None,
            },
            Some(Expr::Identifier(value)) if variable.ends_with("KEYSET_NAME") => {
                Some(value.value.clone())
            }
            _ => None,
        };
        let Some(value) = value.filter(|_| values.len() == 1) else {
            return Err(EncryptError::InvalidKeysetSettingValue { setting: variable }.into());
        };
        let identifier = if variable.ends_with("KEYSET_ID") {
            IdentifiedBy::Uuid(
                uuid::Uuid::parse_str(&value)
                    .map_err(|_| EncryptError::KeysetIdCouldNotBeParsed { id: value })?,
            )
        } else {
            IdentifiedBy::Name(value.into())
        };
        Ok(Some(Self::Keyset(scope, KeysetIdentifier(identifier))))
    }
}

/// The small part of a parsed statement that changes connection settings.
/// Prepared statements and pending executions retain this instead of a SQL AST.
#[derive(Clone, Debug)]
pub enum SettingsCommand {
    Other,
    Begin,
    Commit,
    Rollback,
    Savepoint(String),
    RollbackTo(String),
    Release(String),
    Change(SettingChange),
}

impl SettingsCommand {
    pub fn from_statement(statement: &Statement) -> Self {
        match statement {
            Statement::StartTransaction { .. } => Self::Begin,
            Statement::Commit { .. } => Self::Commit,
            Statement::Rollback {
                savepoint: None, ..
            } => Self::Rollback,
            Statement::Rollback {
                savepoint: Some(name),
                ..
            } => Self::RollbackTo(postgres_identifier(name)),
            Statement::Savepoint { name } => Self::Savepoint(postgres_identifier(name)),
            Statement::ReleaseSavepoint { name } => Self::Release(postgres_identifier(name)),
            _ => SettingChange::parse(statement)
                .ok()
                .flatten()
                .map(Self::Change)
                .unwrap_or(Self::Other),
        }
    }

    pub fn may_change_effective(&self, settings: &ConnectionSettings) -> bool {
        if matches!(
            self,
            Self::Other | Self::Begin | Self::Savepoint(_) | Self::Release(_)
        ) {
            return false;
        }
        let before = super::settings_protocol::EffectiveSettings::new(settings);
        let mut after = settings.clone();
        after.command_succeeded(self, self.success_tag());
        if before != super::settings_protocol::EffectiveSettings::new(&after) {
            return true;
        }
        // An earlier pipelined error can turn COMMIT into ROLLBACK.
        if matches!(self, Self::Commit) {
            let mut aborted = settings.clone();
            aborted.command_succeeded(self, "ROLLBACK");
            return before != super::settings_protocol::EffectiveSettings::new(&aborted);
        }
        false
    }

    pub fn success_tag(&self) -> &'static str {
        if matches!(self, Self::Rollback | Self::RollbackTo(_)) {
            "ROLLBACK"
        } else {
            ""
        }
    }
}

/// How long a `SET CIPHERSTASH.*` value lasts.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingScope {
    /// `SET` or `SET SESSION`: the rest of the connection.
    Session,
    /// `SET LOCAL`: the rest of the current transaction.
    Transaction,
}

impl From<Option<ContextModifier>> for SettingScope {
    fn from(modifier: Option<ContextModifier>) -> Self {
        match modifier {
            Some(ContextModifier::Local) => SettingScope::Transaction,
            // PostgreSQL has no GLOBAL scope; treat it like PostgreSQL treats
            // an unqualified SET rather than inventing a new lifetime.
            Some(ContextModifier::Session) | Some(ContextModifier::Global) | None => {
                SettingScope::Session
            }
        }
    }
}

/// One setting with a session value and an optional transaction override.
#[derive(Clone, Debug)]
struct Scoped<T> {
    session: Option<T>,
    // None inherits the session; Some(None) explicitly selects the default.
    transaction: Option<Option<T>>,
}

impl<T> Default for Scoped<T> {
    fn default() -> Self {
        Self {
            session: None,
            transaction: None,
        }
    }
}

impl<T: Clone> Scoped<T> {
    fn set(&mut self, scope: SettingScope, value: T) {
        self.assign(scope, Some(value));
    }

    fn reset(&mut self, scope: SettingScope) {
        self.assign(scope, None);
    }

    fn assign(&mut self, scope: SettingScope, value: Option<T>) {
        match scope {
            SettingScope::Session => {
                // As in PostgreSQL, a later SET in the same transaction
                // supersedes an earlier SET LOCAL.
                self.session = value;
                self.transaction = None;
            }
            SettingScope::Transaction => self.transaction = Some(value),
        }
    }

    fn current(&self) -> Option<T> {
        self.transaction.as_ref().unwrap_or(&self.session).clone()
    }

    fn end_transaction(&mut self) {
        self.transaction = None;
    }
}

/// The `SET CIPHERSTASH.*` values in effect on one connection.
#[derive(Clone, Debug, Default)]
pub struct ConnectionSettings {
    keyset: Scoped<KeysetIdentifier>,
    unsafe_disable_mapping: Scoped<bool>,
    // Captured lazily before the first confirmed execution. This also covers an
    // implicit extended-protocol transaction whose first command is not BEGIN.
    transaction_start: Option<SettingsSnapshot>,
    failed: bool,
    savepoints: Vec<(String, SettingsSnapshot)>,
}

#[derive(Clone, Debug)]
struct SettingsSnapshot {
    keyset: Scoped<KeysetIdentifier>,
    unsafe_disable_mapping: Scoped<bool>,
}

impl ConnectionSettings {
    /// Applies only a command confirmed successful by PostgreSQL.
    pub fn execution_succeeded(&mut self, statement: &Statement, command_tag: &str) {
        self.command_succeeded(&SettingsCommand::from_statement(statement), command_tag);
    }

    pub fn command_succeeded(&mut self, command: &SettingsCommand, command_tag: &str) {
        self.transaction_start
            .get_or_insert_with(|| SettingsSnapshot {
                keyset: self.keyset.clone(),
                unsafe_disable_mapping: self.unsafe_disable_mapping.clone(),
            });
        match command {
            SettingsCommand::Commit | SettingsCommand::Rollback => {
                if command_tag == "ROLLBACK" {
                    self.restore_transaction();
                }
                self.end_transaction();
            }
            SettingsCommand::Savepoint(name) => self.savepoints.push((
                name.clone(),
                SettingsSnapshot {
                    keyset: self.keyset.clone(),
                    unsafe_disable_mapping: self.unsafe_disable_mapping.clone(),
                },
            )),
            SettingsCommand::RollbackTo(name) => {
                if let Some(index) = self.savepoints.iter().rposition(|(saved, _)| saved == name) {
                    let snapshot = self.savepoints[index].1.clone();
                    self.keyset = snapshot.keyset;
                    self.unsafe_disable_mapping = snapshot.unsafe_disable_mapping;
                    self.savepoints.truncate(index + 1);
                    self.failed = false;
                }
            }
            SettingsCommand::Release(name) => {
                if let Some(index) = self.savepoints.iter().rposition(|(saved, _)| saved == name) {
                    self.savepoints.truncate(index);
                }
            }
            SettingsCommand::Change(change) => match change {
                SettingChange::Keyset(scope, keyset) => self.set_keyset(*scope, keyset.clone()),
                SettingChange::Mapping(scope, disabled) => {
                    self.set_unsafe_disable_mapping(*scope, *disabled)
                }
                SettingChange::DiscardAll => self.reset(),
                SettingChange::ResetAll => {
                    self.keyset = Scoped::default();
                    self.unsafe_disable_mapping = Scoped::default();
                }
                SettingChange::ResetKeyset(scope) => self.keyset.reset(*scope),
                SettingChange::ResetMapping(scope) => self.unsafe_disable_mapping.reset(*scope),
            },
            SettingsCommand::Other | SettingsCommand::Begin => (),
        }
    }

    pub fn execution_failed(&mut self) {
        self.failed = true;
    }

    pub fn ready_for_query(&mut self, status: pg_proto::TransactionStatus) {
        if status == pg_proto::TransactionStatus::Idle {
            if self.failed {
                self.restore_transaction();
            }
            self.end_transaction();
        }
    }

    fn restore_transaction(&mut self) {
        if let Some(snapshot) = self.transaction_start.take() {
            self.keyset = snapshot.keyset;
            self.unsafe_disable_mapping = snapshot.unsafe_disable_mapping;
        }
    }

    /// The keyset selected with `SET CIPHERSTASH.KEYSET_ID` or `KEYSET_NAME`.
    pub fn keyset(&self) -> Option<KeysetIdentifier> {
        self.keyset.current()
    }

    pub fn set_keyset(&mut self, scope: SettingScope, keyset: KeysetIdentifier) {
        self.keyset.set(scope, keyset);
    }

    /// Whether `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true` is in effect.
    pub fn unsafe_disable_mapping(&self) -> bool {
        self.unsafe_disable_mapping.current().unwrap_or(false)
    }

    pub fn set_unsafe_disable_mapping(&mut self, scope: SettingScope, disabled: bool) {
        self.unsafe_disable_mapping.set(scope, disabled);
    }

    /// Completes the current transaction after its commit/rollback has settled.
    pub fn end_transaction(&mut self) {
        self.keyset.end_transaction();
        self.unsafe_disable_mapping.end_transaction();
        self.transaction_start = None;
        self.failed = false;
        self.savepoints.clear();
    }

    /// Returns every value to its default, as `DISCARD ALL` does.
    pub fn reset(&mut self) {
        *self = Self::default();
    }

    /// True when no value has been set.
    pub fn is_default(&self) -> bool {
        self.keyset().is_none() && !self.unsafe_disable_mapping()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use cipherstash_client::IdentifiedBy;
    use uuid::Uuid;

    fn succeeds(settings: &mut ConnectionSettings, sql: &str, tag: &str) {
        let statements = crate::postgresql::parser::SqlParser::parse_statements(sql).unwrap();
        settings.execution_succeeded(&statements[0], tag);
    }

    #[test]
    fn review_r04_known_keyset_setting_never_silently_ignores_value() {
        for sql in [
            "SET CIPHERSTASH.KEYSET_ID = unquoted",
            "SET CIPHERSTASH.KEYSET_NAME = true",
        ] {
            let statements = crate::postgresql::parser::SqlParser::parse_statements(sql).unwrap();
            assert!(
                !matches!(SettingChange::parse(&statements[0]), Ok(None)),
                "{sql}"
            );
        }
    }

    #[test]
    fn local_default_temporarily_overrides_session_values_until_commit_or_rollback() {
        for keyset_setting in ["KEYSET_NAME", "KEYSET_ID"] {
            for completion in ["COMMIT", "ROLLBACK"] {
                let mut settings = ConnectionSettings::default();
                succeeds(
                    &mut settings,
                    "SET CIPHERSTASH.KEYSET_NAME = 'session'",
                    "SET",
                );
                succeeds(
                    &mut settings,
                    "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
                    "SET",
                );
                settings.ready_for_query(pg_proto::TransactionStatus::Idle);
                let session = settings.keyset();
                succeeds(&mut settings, "BEGIN", "BEGIN");
                succeeds(
                    &mut settings,
                    &format!("SET LOCAL CIPHERSTASH.{keyset_setting} TO DEFAULT"),
                    "SET",
                );
                succeeds(
                    &mut settings,
                    "SET LOCAL CIPHERSTASH.UNSAFE_DISABLE_MAPPING TO DEFAULT",
                    "SET",
                );
                assert_eq!(settings.keyset(), None, "{keyset_setting}");
                assert!(!settings.unsafe_disable_mapping());
                succeeds(&mut settings, completion, completion);
                settings.ready_for_query(pg_proto::TransactionStatus::Idle);
                assert_eq!(settings.keyset(), session);
                assert!(settings.unsafe_disable_mapping());
            }
        }
    }

    #[test]
    fn savepoint_rollback_restores_explicit_local_defaults_and_prior_session_values() {
        let mut settings = ConnectionSettings::default();
        succeeds(
            &mut settings,
            "SET CIPHERSTASH.KEYSET_NAME = 'session'",
            "SET",
        );
        succeeds(
            &mut settings,
            "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
            "SET",
        );
        settings.ready_for_query(pg_proto::TransactionStatus::Idle);
        let session = settings.keyset();
        succeeds(&mut settings, "BEGIN", "BEGIN");
        succeeds(&mut settings, "SAVEPOINT before_defaults", "SAVEPOINT");
        succeeds(
            &mut settings,
            "SET LOCAL CIPHERSTASH.KEYSET_ID TO DEFAULT",
            "SET",
        );
        succeeds(
            &mut settings,
            "SET LOCAL CIPHERSTASH.UNSAFE_DISABLE_MAPPING TO DEFAULT",
            "SET",
        );
        succeeds(&mut settings, "SAVEPOINT defaults", "SAVEPOINT");
        succeeds(
            &mut settings,
            "SET CIPHERSTASH.KEYSET_NAME = 'changed'",
            "SET",
        );
        succeeds(
            &mut settings,
            "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
            "SET",
        );
        succeeds(&mut settings, "ROLLBACK TO defaults", "ROLLBACK");
        assert_eq!(settings.keyset(), None);
        assert!(!settings.unsafe_disable_mapping());
        succeeds(&mut settings, "ROLLBACK TO before_defaults", "ROLLBACK");
        assert_eq!(settings.keyset(), session);
        assert!(settings.unsafe_disable_mapping());
        succeeds(&mut settings, "COMMIT", "COMMIT");
        assert_eq!(settings.keyset(), session);
        assert!(settings.unsafe_disable_mapping());
    }

    #[test]
    fn session_default_supersedes_local_values_and_persists_after_commit() {
        for keyset_setting in ["KEYSET_ID", "KEYSET_NAME"] {
            for scope in ["", "SESSION "] {
                let mut settings = ConnectionSettings::default();
                succeeds(
                    &mut settings,
                    "SET CIPHERSTASH.KEYSET_NAME = 'session'",
                    "SET",
                );
                succeeds(
                    &mut settings,
                    "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
                    "SET",
                );
                settings.ready_for_query(pg_proto::TransactionStatus::Idle);
                succeeds(&mut settings, "BEGIN", "BEGIN");
                succeeds(
                    &mut settings,
                    "SET LOCAL CIPHERSTASH.KEYSET_NAME = 'local'",
                    "SET",
                );
                succeeds(
                    &mut settings,
                    "SET LOCAL CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
                    "SET",
                );
                succeeds(
                    &mut settings,
                    &format!("SET {scope}CIPHERSTASH.{keyset_setting} TO DEFAULT"),
                    "SET",
                );
                succeeds(
                    &mut settings,
                    &format!("SET {scope}CIPHERSTASH.UNSAFE_DISABLE_MAPPING TO DEFAULT"),
                    "SET",
                );
                assert!(settings.is_default());
                succeeds(&mut settings, "COMMIT", "COMMIT");
                settings.ready_for_query(pg_proto::TransactionStatus::Idle);
                assert!(settings.is_default());
            }
        }
    }

    #[test]
    fn reset_all_and_individual_resets_preserve_transaction_and_savepoint_snapshots() {
        let mut settings = ConnectionSettings::default();
        succeeds(
            &mut settings,
            "SET CIPHERSTASH.KEYSET_NAME = 'original'",
            "SET",
        );
        succeeds(
            &mut settings,
            "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
            "SET",
        );
        settings.ready_for_query(pg_proto::TransactionStatus::Idle);
        let original = settings.keyset();
        succeeds(&mut settings, "BEGIN", "BEGIN");
        succeeds(&mut settings, "SAVEPOINT before_reset", "SAVEPOINT");
        succeeds(&mut settings, "RESET ALL", "RESET");
        assert!(settings.is_default());
        succeeds(&mut settings, "ROLLBACK TO before_reset", "ROLLBACK");
        assert_eq!(settings.keyset(), original);
        assert!(settings.unsafe_disable_mapping());
        succeeds(&mut settings, "RESET CIPHERSTASH.KEYSET_ID", "RESET");
        assert_eq!(settings.keyset(), None);
        assert!(settings.unsafe_disable_mapping());
        succeeds(
            &mut settings,
            "RESET CIPHERSTASH.UNSAFE_DISABLE_MAPPING",
            "RESET",
        );
        assert!(settings.is_default());
        succeeds(&mut settings, "ROLLBACK", "ROLLBACK");
        assert_eq!(settings.keyset(), original);
        assert!(settings.unsafe_disable_mapping());
    }

    #[test]
    fn reset_all_is_restored_on_implicit_abort_but_persists_on_commit() {
        let mut settings = ConnectionSettings::default();
        succeeds(
            &mut settings,
            "SET CIPHERSTASH.KEYSET_NAME = 'original'",
            "SET",
        );
        settings.ready_for_query(pg_proto::TransactionStatus::Idle);
        let original = settings.keyset();
        succeeds(&mut settings, "RESET ALL", "RESET");
        settings.execution_failed();
        settings.ready_for_query(pg_proto::TransactionStatus::Idle);
        assert_eq!(settings.keyset(), original);
        succeeds(&mut settings, "RESET ALL", "RESET");
        settings.ready_for_query(pg_proto::TransactionStatus::Idle);
        assert!(settings.is_default());
    }

    #[test]
    fn session_changes_are_restored_when_transaction_rolls_back() {
        let mut settings = ConnectionSettings::default();
        succeeds(
            &mut settings,
            "SET CIPHERSTASH.KEYSET_NAME = 'original'",
            "SET",
        );
        settings.ready_for_query(pg_proto::TransactionStatus::Idle);
        let original = settings.keyset();
        succeeds(&mut settings, "BEGIN", "BEGIN");
        succeeds(
            &mut settings,
            "SET CIPHERSTASH.KEYSET_NAME = 'other'",
            "SET",
        );
        succeeds(
            &mut settings,
            "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
            "SET",
        );
        succeeds(&mut settings, "ROLLBACK", "ROLLBACK");
        settings.ready_for_query(pg_proto::TransactionStatus::Idle);
        assert_eq!(settings.keyset(), original);
        assert!(!settings.unsafe_disable_mapping());
    }

    #[test]
    fn failed_implicit_batch_and_commit_of_aborted_transaction_restore_settings() {
        for explicit in [false, true] {
            let mut settings = ConnectionSettings::default();
            succeeds(
                &mut settings,
                "SET CIPHERSTASH.KEYSET_NAME = 'original'",
                "SET",
            );
            settings.ready_for_query(pg_proto::TransactionStatus::Idle);
            let original = settings.keyset();
            if explicit {
                succeeds(&mut settings, "BEGIN", "BEGIN");
            }
            succeeds(
                &mut settings,
                "SET CIPHERSTASH.KEYSET_NAME = 'other'",
                "SET",
            );
            settings.execution_failed();
            if explicit {
                succeeds(&mut settings, "COMMIT", "ROLLBACK");
            }
            settings.ready_for_query(pg_proto::TransactionStatus::Idle);
            assert_eq!(settings.keyset(), original);
        }
    }

    #[test]
    fn chained_transactions_keep_their_own_rollback_baseline_and_local_scope() {
        let mut settings = ConnectionSettings::default();
        succeeds(&mut settings, "BEGIN", "BEGIN");
        succeeds(
            &mut settings,
            "SET CIPHERSTASH.KEYSET_NAME = 'committed'",
            "SET",
        );
        succeeds(&mut settings, "COMMIT AND CHAIN", "COMMIT");
        succeeds(
            &mut settings,
            "SET LOCAL CIPHERSTASH.KEYSET_NAME = 'local'",
            "SET",
        );
        assert_eq!(
            settings.keyset(),
            Some(KeysetIdentifier(IdentifiedBy::Name(
                "local".to_owned().into()
            )))
        );
        succeeds(
            &mut settings,
            "SET CIPHERSTASH.KEYSET_NAME = 'rolled-back'",
            "SET",
        );
        succeeds(&mut settings, "ROLLBACK AND CHAIN", "ROLLBACK");
        assert_eq!(
            settings.keyset(),
            Some(KeysetIdentifier(IdentifiedBy::Name(
                "committed".to_owned().into()
            )))
        );
        succeeds(
            &mut settings,
            "SET LOCAL CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
            "SET",
        );
        assert!(settings.unsafe_disable_mapping());
        succeeds(&mut settings, "COMMIT", "COMMIT");
        settings.ready_for_query(pg_proto::TransactionStatus::Idle);
        assert!(!settings.unsafe_disable_mapping());
    }

    #[test]
    fn nested_duplicate_savepoints_restore_both_scopes_and_allow_repeated_rollback() {
        let mut settings = ConnectionSettings::default();
        succeeds(
            &mut settings,
            "SET CIPHERSTASH.KEYSET_NAME = 'session'",
            "SET",
        );
        settings.ready_for_query(pg_proto::TransactionStatus::Idle);
        let session = settings.keyset();
        succeeds(&mut settings, "BEGIN", "BEGIN");
        succeeds(&mut settings, "SAVEPOINT point", "SAVEPOINT");
        succeeds(
            &mut settings,
            "SET LOCAL CIPHERSTASH.KEYSET_NAME = 'outer-local'",
            "SET",
        );
        let outer = settings.keyset();
        succeeds(&mut settings, "SAVEPOINT point", "SAVEPOINT");
        for _ in 0..2 {
            succeeds(
                &mut settings,
                "SET CIPHERSTASH.KEYSET_NAME = 'inner-session'",
                "SET",
            );
            succeeds(
                &mut settings,
                "SET LOCAL CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true",
                "SET",
            );
            settings.execution_failed();
            succeeds(&mut settings, "ROLLBACK TO POINT", "ROLLBACK");
            assert_eq!(settings.keyset(), outer);
            assert!(!settings.unsafe_disable_mapping());
        }
        succeeds(&mut settings, "RELEASE point", "RELEASE");
        succeeds(&mut settings, "ROLLBACK TO point", "ROLLBACK");
        assert_eq!(settings.keyset(), session);
        succeeds(&mut settings, "COMMIT", "COMMIT");
        settings.ready_for_query(pg_proto::TransactionStatus::Idle);
        assert_eq!(settings.keyset(), session);
    }

    #[test]
    fn released_changes_commit_and_quoted_savepoints_remain_distinct() {
        let mut settings = ConnectionSettings::default();
        succeeds(&mut settings, "BEGIN", "BEGIN");
        succeeds(&mut settings, "SAVEPOINT point", "SAVEPOINT");
        succeeds(
            &mut settings,
            "SET CIPHERSTASH.KEYSET_NAME = 'session'",
            "SET",
        );
        let session = settings.keyset();
        succeeds(&mut settings, "SAVEPOINT \"POINT\"", "SAVEPOINT");
        succeeds(
            &mut settings,
            "SET LOCAL CIPHERSTASH.KEYSET_NAME = 'local'",
            "SET",
        );
        succeeds(&mut settings, "ROLLBACK TO \"POINT\"", "ROLLBACK");
        assert_eq!(settings.keyset(), session);
        succeeds(&mut settings, "RELEASE point", "RELEASE");
        assert_eq!(settings.keyset(), session);
        succeeds(&mut settings, "COMMIT", "COMMIT");
        settings.ready_for_query(pg_proto::TransactionStatus::Idle);
        assert_eq!(settings.keyset(), session);
    }

    fn keyset() -> KeysetIdentifier {
        KeysetIdentifier(IdentifiedBy::Uuid(Uuid::new_v4()))
    }

    #[test]
    fn production_parser_accepts_keyset_name_and_id_forms() {
        for (sql, expected) in [
            (
                "SET CIPHERSTASH.KEYSET_NAME = 'tenant'",
                IdentifiedBy::Name("tenant".to_owned().into()),
            ),
            (
                "SET SESSION CIPHERSTASH.KEYSET_NAME = 'tenant'",
                IdentifiedBy::Name("tenant".to_owned().into()),
            ),
            (
                "SET CIPHERSTASH.KEYSET_NAME TO 'tenant'",
                IdentifiedBy::Name("tenant".to_owned().into()),
            ),
            (
                "SET CIPHERSTASH.KEYSET_NAME = tenant",
                IdentifiedBy::Name("tenant".to_owned().into()),
            ),
            (
                "SET CIPHERSTASH.KEYSET_NAME = 12345",
                IdentifiedBy::Name("12345".to_owned().into()),
            ),
            (
                "SET CIPHERSTASH.KEYSET_ID = '7d4cbd7f-ba0d-4985-9ed2-ebe2ffe77590'",
                IdentifiedBy::Uuid(
                    Uuid::parse_str("7d4cbd7f-ba0d-4985-9ed2-ebe2ffe77590").unwrap(),
                ),
            ),
            (
                "SET SESSION CIPHERSTASH.KEYSET_ID TO '7d4cbd7f-ba0d-4985-9ed2-ebe2ffe77590'",
                IdentifiedBy::Uuid(
                    Uuid::parse_str("7d4cbd7f-ba0d-4985-9ed2-ebe2ffe77590").unwrap(),
                ),
            ),
        ] {
            let mut settings = ConnectionSettings::default();
            succeeds(&mut settings, sql, "SET");
            assert_eq!(settings.keyset(), Some(KeysetIdentifier(expected)), "{sql}");
        }
    }

    #[test]
    fn production_parser_rejects_invalid_uuid_and_ignores_unrecognised_values() {
        let parse = |sql: &str| {
            sqltk::parser::parser::Parser::parse_sql(
                &sqltk::parser::dialect::PostgreSqlDialect {},
                sql,
            )
            .unwrap()
            .remove(0)
        };
        for sql in [
            "SET CIPHERSTASH.KEYSET_ID = 'invalid'",
            "SET CIPHERSTASH.KEYSET_ID = 123",
            "SET CIPHERSTASH.KEYSET_ID = unquoted",
            "SET CIPHERSTASH.KEYSET_NAME = test-keyset",
        ] {
            assert!(SettingChange::parse(&parse(sql)).is_err(), "{sql}");
        }
        for sql in [
            "SET CIPHERSTASH.UNKNOWN = 'value'",
            "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = 1",
            "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = '1'",
            "SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = t",
            "DISCARD PLANS",
            "DISCARD TEMP",
        ] {
            assert!(
                SettingChange::parse(&parse(sql)).unwrap().is_none(),
                "{sql}"
            );
        }
    }

    #[test]
    fn scope_follows_postgres_set_modifiers() {
        assert_eq!(SettingScope::from(None), SettingScope::Session);
        assert_eq!(
            SettingScope::from(Some(ContextModifier::Session)),
            SettingScope::Session
        );
        assert_eq!(
            SettingScope::from(Some(ContextModifier::Local)),
            SettingScope::Transaction
        );
    }

    #[test]
    fn session_keyset_survives_the_end_of_a_transaction() {
        let mut settings = ConnectionSettings::default();
        let session = keyset();
        settings.set_keyset(SettingScope::Session, session.clone());

        settings.end_transaction();

        assert_eq!(settings.keyset(), Some(session));
    }

    #[test]
    fn local_keyset_ends_with_the_transaction() {
        let mut settings = ConnectionSettings::default();
        let local = keyset();
        settings.set_keyset(SettingScope::Transaction, local.clone());
        assert_eq!(settings.keyset(), Some(local));

        settings.end_transaction();

        assert_eq!(settings.keyset(), None);
    }

    #[test]
    fn local_keyset_overrides_and_then_restores_the_session_keyset() {
        let mut settings = ConnectionSettings::default();
        let session = keyset();
        let local = keyset();
        settings.set_keyset(SettingScope::Session, session.clone());
        settings.set_keyset(SettingScope::Transaction, local.clone());
        assert_eq!(settings.keyset(), Some(local));

        settings.end_transaction();

        assert_eq!(settings.keyset(), Some(session));
    }

    #[test]
    fn session_set_after_local_set_supersedes_it() {
        let mut settings = ConnectionSettings::default();
        let local = keyset();
        let session = keyset();
        settings.set_keyset(SettingScope::Transaction, local);
        settings.set_keyset(SettingScope::Session, session.clone());
        assert_eq!(settings.keyset(), Some(session.clone()));

        settings.end_transaction();

        assert_eq!(settings.keyset(), Some(session));
    }

    #[test]
    fn local_unsafe_disable_mapping_ends_with_the_transaction() {
        let mut settings = ConnectionSettings::default();
        settings.set_unsafe_disable_mapping(SettingScope::Transaction, true);
        assert!(settings.unsafe_disable_mapping());

        settings.end_transaction();

        assert!(!settings.unsafe_disable_mapping());
    }

    #[test]
    fn reset_clears_session_and_local_values() {
        let mut settings = ConnectionSettings::default();
        settings.set_keyset(SettingScope::Session, keyset());
        settings.set_keyset(SettingScope::Transaction, keyset());
        settings.set_unsafe_disable_mapping(SettingScope::Session, true);
        assert!(!settings.is_default());

        settings.reset();

        assert!(settings.is_default());
        assert_eq!(settings.keyset(), None);
        assert!(!settings.unsafe_disable_mapping());
    }
}
