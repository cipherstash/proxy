//! Settings effects and the speculative metadata journal for one connection.
use super::{
    settings::{ConnectionSettings, SettingChange, SettingsCommand},
    KeysetIdentifier, SessionId,
};
use crate::postgresql::{Name, OperationId};
use sqltk::parser::ast::Statement;
use std::{
    collections::{BTreeMap, HashMap, VecDeque},
    sync::Arc,
};

/// Why a prepared statement or portal no longer matches the effective settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SettingsRefusal {
    MappingChanged,
    KeysetChanged,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EffectiveSettings {
    keyset: Option<KeysetIdentifier>,
    mapping_disabled: bool,
}
impl EffectiveSettings {
    pub fn new(settings: &ConnectionSettings) -> Self {
        Self {
            keyset: settings.keyset(),
            mapping_disabled: settings.unsafe_disable_mapping(),
        }
    }
    fn refusal(&self, current: &Self, keyset_sensitive: bool) -> Option<SettingsRefusal> {
        if self.mapping_disabled != current.mapping_disabled {
            Some(SettingsRefusal::MappingChanged)
        } else if keyset_sensitive && self.keyset != current.keyset {
            Some(SettingsRefusal::KeysetChanged)
        } else {
            None
        }
    }
}

/// Projects a simple-query batch over a copy of the confirmed settings, yielding
/// each statement, its settings command, and whether it may change the
/// effective settings after the statements before it succeed.
pub(super) fn project_settings_effects<'a>(
    settings: &ConnectionSettings,
    statements: &'a [Statement],
) -> impl Iterator<Item = (&'a Statement, SettingsCommand, bool)> + 'a {
    let mut projected = settings.clone();
    statements.iter().map(move |statement| {
        let command = SettingsCommand::from_statement(statement);
        let changes = command.may_change_effective(&projected);
        projected.command_succeeded(&command, command.success_tag());
        (statement, command, changes)
    })
}

#[derive(Clone, Debug)]
struct Prepared {
    operation: OperationId,
    command: SettingsCommand,
    needs_mapping: bool,
    settings: EffectiveSettings,
    literal_sensitive: bool,
}
#[derive(Clone, Debug)]
struct Bound {
    operation: OperationId,
    prepared: Prepared,
    settings: EffectiveSettings,
    encrypted_values: bool,
}
#[derive(Debug)]
struct Pending {
    commands: VecDeque<SettingsCommand>,
    barrier: bool,
    until_ready: bool,
}

/// Settings and encryption metadata share one speculative operation journal.
/// Context owns live analysed objects; this journal owns their rollback snapshots.
#[derive(Debug, Default)]
pub(super) struct MetadataSnapshot {
    pub statement: Option<Arc<super::Statement>>,
    pub statement_metrics: Option<SessionId>,
    pub portal: Option<Arc<super::Portal>>,
    pub portal_operation: Option<OperationId>,
    pub suspended: Option<(OperationId, super::ExecuteContext)>,
}
impl MetadataSnapshot {
    pub fn metrics(&self) -> impl Iterator<Item = SessionId> {
        self.statement_metrics
            .into_iter()
            .chain(self.portal.as_ref().and_then(|portal| portal.session_id()))
            .chain(
                self.suspended
                    .as_ref()
                    .and_then(|(_, execute)| execute.session_id()),
            )
    }
}
#[derive(Debug)]
enum Previous {
    Statement(Option<Prepared>),
    Portal(Option<Bound>),
}
#[derive(Debug)]
struct Mutation {
    name: Name,
    previous: Previous,
    metadata: MetadataSnapshot,
    close: bool,
}
#[derive(Debug)]
pub(super) struct RestoreMetadata {
    pub name: Name,
    pub statement: bool,
    pub metadata: MetadataSnapshot,
}
#[derive(Debug, Default)]
pub struct SettingsProtocol {
    prepared: HashMap<Name, Prepared>,
    portals: HashMap<Name, Bound>,
    journal: BTreeMap<OperationId, Mutation>,
    pending: BTreeMap<OperationId, Pending>,
    boundaries: Vec<OperationId>,
    settings_since_boundary: bool,
    discarding_until_sync: bool,
}

impl SettingsProtocol {
    pub(super) fn start_prepare(
        &mut self,
        operation: OperationId,
        name: Name,
        metadata: MetadataSnapshot,
    ) {
        self.journal.insert(
            operation,
            Mutation {
                previous: Previous::Statement(self.prepared.get(&name).cloned()),
                name,
                metadata,
                close: false,
            },
        );
    }
    pub fn references_metrics(&self, id: SessionId) -> bool {
        self.journal
            .values()
            .any(|mutation| mutation.metadata.metrics().any(|saved| saved == id))
    }
    pub fn prepare(
        &mut self,
        operation: OperationId,
        name: Name,
        statement: &Statement,
        settings: EffectiveSettings,
    ) {
        let previous = self.prepared.insert(
            name.clone(),
            Prepared {
                operation,
                command: SettingsCommand::from_statement(statement),
                needs_mapping: eql_mapper::requires_type_check(statement),
                settings,
                literal_sensitive: false,
            },
        );
        self.journal.entry(operation).or_insert(Mutation {
            name,
            previous: Previous::Statement(previous),
            metadata: MetadataSnapshot::default(),
            close: false,
        });
    }
    pub fn capture_literals(&mut self, name: &Name, sensitive: bool) {
        if let Some(prepared) = self.prepared.get_mut(name) {
            prepared.literal_sensitive = sensitive;
        }
    }
    pub fn bind(
        &mut self,
        operation: OperationId,
        portal: Name,
        name: &Name,
        settings: EffectiveSettings,
        encrypted_values: bool,
        metadata: MetadataSnapshot,
    ) -> Result<(), SettingsRefusal> {
        let previous = self.portals.remove(&portal);
        self.journal.insert(
            operation,
            Mutation {
                name: portal.clone(),
                previous: Previous::Portal(previous),
                metadata,
                close: false,
            },
        );
        let Some(prepared) = self.prepared.get(name).cloned() else {
            return Ok(());
        };
        if prepared.needs_mapping {
            if let Some(refusal) = prepared
                .settings
                .refusal(&settings, prepared.literal_sensitive)
            {
                return Err(refusal);
            }
        }
        self.portals.insert(
            portal,
            Bound {
                operation,
                prepared,
                settings,
                encrypted_values,
            },
        );
        Ok(())
    }
    pub fn execute(
        &mut self,
        operation: OperationId,
        portal: &Name,
        connection: &ConnectionSettings,
    ) -> Result<bool, SettingsRefusal> {
        let settings = EffectiveSettings::new(connection);
        let Some(bound) = self.portals.get(portal) else {
            return Ok(false);
        };
        if bound.prepared.needs_mapping {
            if let Some(refusal) = bound.settings.refusal(
                &settings,
                bound.encrypted_values || bound.prepared.literal_sensitive,
            ) {
                return Err(refusal);
            }
        }
        let command = bound.prepared.command.clone();
        let barrier = command.may_change_effective(connection);
        self.pending.insert(
            operation,
            Pending {
                commands: VecDeque::from([command]),
                barrier,
                until_ready: false,
            },
        );
        Ok(barrier)
    }
    pub fn simple_query(
        &mut self,
        operation: OperationId,
        statements: &[Statement],
        settings: &ConnectionSettings,
    ) -> bool {
        let mut barrier = false;
        let commands: VecDeque<_> = project_settings_effects(settings, statements)
            .map(|(_, command, changes)| {
                barrier |= changes;
                command
            })
            .collect();
        self.pending.insert(
            operation,
            Pending {
                commands,
                barrier,
                until_ready: true,
            },
        );
        self.boundary(operation);
        barrier
    }
    pub fn succeeded(&mut self, operation: OperationId) -> Option<SettingsCommand> {
        let pending = self.pending.get_mut(&operation)?;
        let command = pending.commands.pop_front();
        self.settings_since_boundary |= pending.barrier;
        if !pending.until_ready && pending.commands.is_empty() {
            self.pending.remove(&operation);
        }
        if matches!(
            command,
            Some(SettingsCommand::Change(SettingChange::DiscardAll))
        ) {
            self.prepared
                .retain(|_, prepared| prepared.operation > operation);
            self.portals.retain(|_, bound| bound.operation > operation);
            self.journal.retain(|id, _| *id > operation);
            for mutation in self.journal.values_mut() {
                let expired = match &mutation.previous {
                    Previous::Statement(previous) => previous
                        .as_ref()
                        .is_some_and(|prepared| prepared.operation <= operation),
                    Previous::Portal(previous) => previous
                        .as_ref()
                        .is_some_and(|bound| bound.operation <= operation),
                };
                if expired {
                    mutation.previous = match mutation.previous {
                        Previous::Statement(_) => Previous::Statement(None),
                        Previous::Portal(_) => Previous::Portal(None),
                    };
                    mutation.metadata = MetadataSnapshot::default();
                }
            }
        }
        command
    }
    pub(super) fn prepared_or_bound(&mut self, operation: OperationId) -> Option<MetadataSnapshot> {
        self.journal
            .remove(&operation)
            .map(|mutation| mutation.metadata)
    }
    /// Rolls back the failed operation's skipped mutations. Returns the restored
    /// metadata and the metrics scopes whose only journal reference was dropped.
    pub(super) fn failed(
        &mut self,
        operation: Option<OperationId>,
    ) -> (Vec<RestoreMetadata>, Vec<SessionId>) {
        let Some(operation) = operation else {
            return (Vec::new(), Vec::new());
        };
        let boundary = self
            .boundaries
            .iter()
            .copied()
            .find(|boundary| *boundary >= operation);
        self.discarding_until_sync |= boundary.is_none();
        let skipped =
            |id: &OperationId| *id >= operation && boundary.is_none_or(|boundary| *id <= boundary);
        // PostgreSQL applies a failed barrier's rollback (e.g. COMMIT in an
        // aborted transaction) at ReadyForQuery; the next Sync must wait for it.
        self.settings_since_boundary |= self
            .pending
            .iter()
            .any(|(id, pending)| skipped(id) && pending.barrier);
        self.pending.retain(|id, pending| {
            if !skipped(id) {
                return true;
            }
            if pending.until_ready && pending.barrier {
                pending.commands.clear();
                true
            } else {
                false
            }
        });
        let mutations: Vec<_> = self
            .journal
            .keys()
            .filter(|id| skipped(id))
            .copied()
            .rev()
            .collect();
        let mut restored = Vec::new();
        let mut released = Vec::new();
        for id in mutations {
            let Mutation {
                name,
                mut previous,
                mut metadata,
                close,
            } = self.journal.remove(&id).unwrap();
            // A rejected unnamed Parse/Bind destroys its previous unnamed object;
            // a skipped later message or Close leaves that object untouched.
            let retain_previous = !name.is_empty() || id != operation || close;
            let statement = matches!(previous, Previous::Statement(_));
            if !retain_previous {
                previous = if statement {
                    Previous::Statement(None)
                } else {
                    Previous::Portal(None)
                };
                released.extend(std::mem::take(&mut metadata).metrics());
            }
            // Later pipelined batches may have already replaced this name.
            // Rebase their rollback snapshot instead of clobbering the live object.
            if let Some((_, later)) = self
                .journal
                .range_mut((std::ops::Bound::Excluded(id), std::ops::Bound::Unbounded))
                .find(|(_, mutation)| {
                    mutation.name == name
                        && matches!(mutation.previous, Previous::Statement(_)) == statement
                })
            {
                later.previous = previous;
                released.extend(std::mem::replace(&mut later.metadata, metadata).metrics());
                continue;
            }
            match previous {
                Previous::Statement(previous) => {
                    self.prepared.remove(&name);
                    if retain_previous {
                        if let Some(previous) = previous {
                            self.prepared.insert(name.clone(), previous);
                        }
                    }
                }
                Previous::Portal(previous) => {
                    self.portals.remove(&name);
                    if retain_previous {
                        if let Some(previous) = previous {
                            self.portals.insert(name.clone(), previous);
                        }
                    }
                }
            }
            restored.push(RestoreMetadata {
                name,
                statement,
                metadata,
            });
        }
        (restored, released)
    }
    pub fn discarding(&self) -> bool {
        self.discarding_until_sync
    }
    pub fn boundary(&mut self, operation: OperationId) {
        self.boundaries.push(operation);
    }
    pub fn sync(&mut self, operation: OperationId) {
        self.discarding_until_sync = false;
        self.boundary(operation);
        if self.settings_since_boundary || self.in_flight() {
            self.pending.insert(
                operation,
                Pending {
                    commands: VecDeque::new(),
                    barrier: true,
                    until_ready: true,
                },
            );
        }
    }
    pub fn ready(&mut self, operation: Option<OperationId>, idle: bool) -> Vec<SessionId> {
        let mut retired_metrics = Vec::new();
        self.settings_since_boundary = false;
        // PostgreSQL only reports readiness once it has stopped discarding.
        self.discarding_until_sync = false;
        if let Some(operation) = operation {
            self.pending.retain(|id, _| *id > operation);
            self.boundaries.retain(|id| *id > operation);
            self.journal.retain(|id, mutation| {
                if *id > operation {
                    true
                } else {
                    retired_metrics.extend(mutation.metadata.metrics());
                    false
                }
            });
            if idle {
                self.portals.retain(|_, bound| bound.operation > operation);
                for mutation in self.journal.values_mut() {
                    if matches!(&mutation.previous, Previous::Portal(Some(bound)) if bound.operation <= operation)
                    {
                        mutation.previous = Previous::Portal(None);
                        retired_metrics.extend(mutation.metadata.metrics());
                        mutation.metadata = MetadataSnapshot::default();
                    }
                }
            }
        }
        retired_metrics
    }
    pub fn has_statement(&self, name: &Name) -> bool {
        self.prepared.contains_key(name)
    }
    pub fn has_portal(&self, name: &Name) -> bool {
        self.portals.contains_key(name)
    }
    pub fn in_flight(&self) -> bool {
        self.pending.values().any(|pending| pending.barrier)
    }
    pub(super) fn close(
        &mut self,
        operation: OperationId,
        name: Name,
        target: pg_proto::DescribeTarget,
        metadata: MetadataSnapshot,
    ) {
        let previous = match target {
            pg_proto::DescribeTarget::Statement => Previous::Statement(self.prepared.remove(&name)),
            pg_proto::DescribeTarget::Portal => Previous::Portal(self.portals.remove(&name)),
        };
        self.journal.insert(
            operation,
            Mutation {
                name,
                previous,
                metadata,
                close: true,
            },
        );
    }
}

#[cfg(test)]
mod protocol_regressions {
    use super::*;
    use crate::postgresql::{parser::SqlParser, test_operation_id as operation_id};
    fn prepare(protocol: &mut SettingsProtocol, name: &Name) {
        let operation = operation_id();
        protocol.prepare(
            operation,
            name.clone(),
            &SqlParser::parse_statement("SELECT 1").unwrap(),
            EffectiveSettings::new(&ConnectionSettings::default()),
        );
        protocol.prepared_or_bound(operation);
    }
    fn named_prepare(protocol: &mut SettingsProtocol, name: &Name, savepoint: &str) -> OperationId {
        let operation = operation_id();
        protocol.prepare(
            operation,
            name.clone(),
            &SqlParser::parse_statement(&format!("SAVEPOINT {savepoint}")).unwrap(),
            EffectiveSettings::new(&ConnectionSettings::default()),
        );
        operation
    }

    #[test]
    fn later_batch_preparation_survives_failure_and_restores_rebased_snapshot() {
        let mut protocol = SettingsProtocol::default();
        let name = Name::from_static(b"statement");
        let original = named_prepare(&mut protocol, &name, "original");
        protocol.prepared_or_bound(original);
        let rejected = named_prepare(&mut protocol, &name, "rejected");
        protocol.sync(operation_id());
        let later = named_prepare(&mut protocol, &name, "later");
        protocol.failed(Some(rejected));
        assert!(!protocol.discarding());
        assert!(
            matches!(&protocol.prepared[&name].command, SettingsCommand::Savepoint(name) if name == "later")
        );
        protocol.failed(Some(later));
        assert!(
            matches!(&protocol.prepared[&name].command, SettingsCommand::Savepoint(name) if name == "original")
        );
    }

    #[test]
    fn later_batch_binding_survives_failure_and_restores_rebased_snapshot() {
        let mut protocol = SettingsProtocol::default();
        let settings = EffectiveSettings::new(&ConnectionSettings::default());
        let name = Name::from_static(b"statement");
        let portal = Name::from_static(b"portal");
        let original = named_prepare(&mut protocol, &name, "original");
        protocol.prepared_or_bound(original);
        let original_bind = operation_id();
        protocol
            .bind(
                original_bind,
                portal.clone(),
                &name,
                settings.clone(),
                false,
                Default::default(),
            )
            .unwrap();
        protocol.prepared_or_bound(original_bind);
        let rejected = operation_id();
        protocol
            .bind(
                rejected,
                portal.clone(),
                &name,
                settings.clone(),
                false,
                Default::default(),
            )
            .unwrap();
        protocol.sync(operation_id());
        let later = operation_id();
        protocol
            .bind(
                later,
                portal.clone(),
                &name,
                settings,
                false,
                Default::default(),
            )
            .unwrap();
        protocol.failed(Some(rejected));
        assert_eq!(protocol.portals[&portal].operation, later);
        protocol.failed(Some(later));
        assert_eq!(protocol.portals[&portal].operation, original_bind);
    }

    #[test]
    fn discard_preserves_later_preparation_but_expires_its_rollback_snapshot() {
        let mut protocol = SettingsProtocol::default();
        let name = Name::from_static(b"statement");
        let original = named_prepare(&mut protocol, &name, "original");
        protocol.prepared_or_bound(original);
        let discard = operation_id();
        protocol.simple_query(
            discard,
            &SqlParser::parse_statements("DISCARD ALL").unwrap(),
            &ConnectionSettings::default(),
        );
        let later = named_prepare(&mut protocol, &name, "later");
        protocol.succeeded(discard);
        assert!(
            matches!(&protocol.prepared[&name].command, SettingsCommand::Savepoint(name) if name == "later")
        );
        assert!(protocol.journal.contains_key(&later));
        protocol.failed(Some(later));
        assert!(!protocol.prepared.contains_key(&name));
    }

    #[test]
    fn idle_boundary_expires_previous_transaction_portal_rollback_snapshot() {
        let mut protocol = SettingsProtocol::default();
        let name = Name::from_static(b"statement");
        let portal = Name::from_static(b"portal");
        let settings = EffectiveSettings::new(&ConnectionSettings::default());
        prepare(&mut protocol, &name);
        let old = operation_id();
        protocol
            .bind(
                old,
                portal.clone(),
                &name,
                settings.clone(),
                false,
                Default::default(),
            )
            .unwrap();
        protocol.prepared_or_bound(old);
        let sync = operation_id();
        protocol.sync(sync);
        let later = operation_id();
        protocol
            .bind(
                later,
                portal.clone(),
                &name,
                settings,
                false,
                Default::default(),
            )
            .unwrap();
        protocol.ready(Some(sync), true);
        protocol.failed(Some(later));
        assert!(!protocol.portals.contains_key(&portal));
    }

    #[test]
    fn close_parse_bind_can_recover_before_replies() {
        let mut protocol = SettingsProtocol::default();
        let name = Name::from_static(b"statement");
        prepare(&mut protocol, &name);
        let rejected = operation_id();
        protocol.prepare(
            rejected,
            name.clone(),
            &SqlParser::parse_statement("SELECT 2").unwrap(),
            EffectiveSettings::new(&ConnectionSettings::default()),
        );
        protocol.failed(Some(rejected));
        protocol.ready(Some(operation_id()), true);
        protocol.close(
            operation_id(),
            name.clone(),
            pg_proto::DescribeTarget::Statement,
            Default::default(),
        );
        protocol.prepare(
            operation_id(),
            name.clone(),
            &SqlParser::parse_statement("SELECT 3").unwrap(),
            EffectiveSettings::new(&ConnectionSettings::default()),
        );
        protocol
            .bind(
                operation_id(),
                Name::new(),
                &name,
                EffectiveSettings::new(&ConnectionSettings::default()),
                false,
                Default::default(),
            )
            .unwrap();
    }
    #[test]
    fn transaction_end_expires_invalidated_portals() {
        let mut protocol = SettingsProtocol::default();
        let name = Name::from_static(b"statement");
        let portal = Name::from_static(b"portal");
        let settings = EffectiveSettings::new(&ConnectionSettings::default());
        prepare(&mut protocol, &name);
        let first = operation_id();
        protocol
            .bind(
                first,
                portal.clone(),
                &name,
                settings.clone(),
                false,
                Default::default(),
            )
            .unwrap();
        protocol.prepared_or_bound(first);
        let rejected = operation_id();
        protocol
            .bind(
                rejected,
                portal.clone(),
                &name,
                settings.clone(),
                false,
                Default::default(),
            )
            .unwrap();
        protocol.failed(Some(rejected));
        protocol.ready(Some(operation_id()), true);
        protocol
            .bind(
                operation_id(),
                portal.clone(),
                &name,
                settings.clone(),
                false,
                Default::default(),
            )
            .unwrap();
        assert_eq!(
            protocol.execute(operation_id(), &portal, &ConnectionSettings::default()),
            Ok(false)
        );
    }
    #[test]
    fn plain_transaction_control_does_not_require_drain() {
        for sql in ["BEGIN", "SAVEPOINT s", "RELEASE SAVEPOINT s"] {
            let mut protocol = SettingsProtocol::default();
            let statements = SqlParser::parse_statements(sql).unwrap();
            assert!(
                !protocol.simple_query(operation_id(), &statements, &ConnectionSettings::default()),
                "{sql}"
            );
        }
    }

    #[test]
    fn readiness_ends_discard_after_failure_without_recorded_boundary() {
        let mut protocol = SettingsProtocol::default();
        // A FunctionCall or configuration-disabled Query records no boundary;
        // PostgreSQL never discards after it, and ReadyForQuery proves readiness.
        let failed = operation_id();
        protocol.failed(Some(failed));
        protocol.ready(Some(failed), true);
        assert!(!protocol.discarding());
    }

    #[test]
    fn execute_refuses_encrypted_portal_after_keyset_changes() {
        let mut protocol = SettingsProtocol::default();
        let name = Name::from_static(b"insert");
        let parse = operation_id();
        protocol.prepare(
            parse,
            name.clone(),
            &SqlParser::parse_statement("INSERT INTO encrypted (secret) VALUES ($1)").unwrap(),
            EffectiveSettings::new(&ConnectionSettings::default()),
        );
        protocol.prepared_or_bound(parse);
        let mut tenant_b = ConnectionSettings::default();
        tenant_b.execution_succeeded(
            &SqlParser::parse_statement("SET CIPHERSTASH.KEYSET_NAME = 'tenant-b'").unwrap(),
            "SET",
        );
        for (portal, encrypted_values, expected) in [
            (
                Name::from_static(b"encrypted"),
                true,
                Err(SettingsRefusal::KeysetChanged),
            ),
            (Name::from_static(b"plain"), false, Ok(false)),
        ] {
            protocol
                .bind(
                    operation_id(),
                    portal.clone(),
                    &name,
                    EffectiveSettings::new(&ConnectionSettings::default()),
                    encrypted_values,
                    Default::default(),
                )
                .unwrap();
            assert_eq!(
                protocol.execute(operation_id(), &portal, &tenant_b),
                expected
            );
        }
    }

    #[test]
    fn bind_refuses_literal_sensitive_statement_after_keyset_changes() {
        let mut protocol = SettingsProtocol::default();
        let name = Name::from_static(b"insert_literal");
        let parse = operation_id();
        protocol.prepare(
            parse,
            name.clone(),
            &SqlParser::parse_statement("INSERT INTO encrypted (secret) VALUES ('literal')")
                .unwrap(),
            EffectiveSettings::new(&ConnectionSettings::default()),
        );
        protocol.capture_literals(&name, true);
        protocol.prepared_or_bound(parse);
        let mut tenant_b = ConnectionSettings::default();
        tenant_b.execution_succeeded(
            &SqlParser::parse_statement("SET CIPHERSTASH.KEYSET_NAME = 'tenant-b'").unwrap(),
            "SET",
        );
        let bind = |protocol: &mut SettingsProtocol, settings: &ConnectionSettings| {
            protocol.bind(
                operation_id(),
                Name::new(),
                &name,
                EffectiveSettings::new(settings),
                false,
                Default::default(),
            )
        };
        assert_eq!(
            bind(&mut protocol, &tenant_b),
            Err(SettingsRefusal::KeysetChanged)
        );
        assert!(!protocol.has_portal(&Name::new()));
        assert_eq!(bind(&mut protocol, &ConnectionSettings::default()), Ok(()));
    }

    #[test]
    fn sync_keeps_successful_settings_change_in_flight_until_ready() {
        let mut protocol = SettingsProtocol::default();
        let name = Name::from_static(b"keyset");
        let parse = operation_id();
        protocol.prepare(
            parse,
            name.clone(),
            &SqlParser::parse_statement("SET CIPHERSTASH.KEYSET_NAME = 'tenant'").unwrap(),
            EffectiveSettings::new(&ConnectionSettings::default()),
        );
        protocol.prepared_or_bound(parse);
        let bind = operation_id();
        protocol
            .bind(
                bind,
                name.clone(),
                &name,
                EffectiveSettings::new(&ConnectionSettings::default()),
                false,
                Default::default(),
            )
            .unwrap();
        protocol.prepared_or_bound(bind);
        let execute = operation_id();
        assert_eq!(
            protocol.execute(execute, &name, &ConnectionSettings::default()),
            Ok(true)
        );
        assert!(protocol.in_flight());
        assert!(protocol.succeeded(execute).is_some());
        let sync = operation_id();
        protocol.sync(sync);
        assert!(protocol.in_flight());
        protocol.ready(Some(sync), true);
        assert!(!protocol.in_flight());
    }

    #[test]
    fn failed_simple_query_settings_change_stays_in_flight_until_ready() {
        let mut protocol = SettingsProtocol::default();
        let query = operation_id();
        assert!(protocol.simple_query(
            query,
            &SqlParser::parse_statements("SET CIPHERSTASH.KEYSET_NAME = 'tenant'").unwrap(),
            &ConnectionSettings::default(),
        ));
        protocol.failed(Some(query));
        assert!(protocol.in_flight());
        protocol.ready(Some(query), true);
        assert!(!protocol.in_flight());
    }

    #[test]
    fn failed_extended_commit_keeps_rolled_back_keyset_in_flight_until_ready() {
        let parse_sql = |sql| SqlParser::parse_statement(sql).unwrap();
        // BEGIN and the keyset change ran in earlier, already-ready batches.
        let mut connection = ConnectionSettings::default();
        connection.execution_succeeded(&parse_sql("BEGIN"), "BEGIN");
        connection.execution_succeeded(&parse_sql("SET CIPHERSTASH.KEYSET_NAME = 'other'"), "SET");
        let mut protocol = SettingsProtocol::default();
        let name = Name::from_static(b"commit");
        let parse = operation_id();
        protocol.prepare(
            parse,
            name.clone(),
            &parse_sql("COMMIT"),
            EffectiveSettings::new(&connection),
        );
        protocol.prepared_or_bound(parse);
        let bind = operation_id();
        protocol
            .bind(
                bind,
                Name::new(),
                &name,
                EffectiveSettings::new(&connection),
                false,
                Default::default(),
            )
            .unwrap();
        protocol.prepared_or_bound(bind);
        // Pipelined: Execute(COMMIT), Sync, Bind+Execute(encrypted INSERT).
        let commit = operation_id();
        assert_eq!(
            protocol.execute(commit, &Name::new(), &connection),
            Ok(true)
        );
        // COMMIT's ErrorResponse is drained before the client's Sync arrives.
        protocol.failed(Some(commit));
        connection.execution_failed();
        protocol.sync(operation_id());
        // PostgreSQL rolled the keyset back, but Context restores it only at
        // ReadyForQuery: the following Bind must wait for readiness.
        assert!(protocol.in_flight());
    }
}
