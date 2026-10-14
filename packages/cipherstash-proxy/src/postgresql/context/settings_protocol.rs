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
    ) -> bool {
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
            return true;
        };
        if prepared.needs_mapping
            && (prepared.settings.mapping_disabled != settings.mapping_disabled
                || (prepared.literal_sensitive && prepared.settings.keyset != settings.keyset))
        {
            return false;
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
        true
    }
    pub fn execute(
        &mut self,
        operation: OperationId,
        portal: &Name,
        connection: &ConnectionSettings,
    ) -> Result<bool, ()> {
        let settings = EffectiveSettings::new(connection);
        let Some(bound) = self.portals.get(portal) else {
            return Ok(false);
        };
        if bound.prepared.needs_mapping
            && (bound.settings.mapping_disabled != settings.mapping_disabled
                || ((bound.encrypted_values || bound.prepared.literal_sensitive)
                    && bound.settings.keyset != settings.keyset))
        {
            return Err(());
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
        let commands: VecDeque<_> = statements
            .iter()
            .map(SettingsCommand::from_statement)
            .collect();
        let mut projected = settings.clone();
        let mut barrier = false;
        for command in &commands {
            barrier |= command.may_change_effective(&projected);
            projected.command_succeeded(command, command.success_tag());
        }
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
    pub(super) fn failed(&mut self, operation: Option<OperationId>) -> Vec<RestoreMetadata> {
        let Some(operation) = operation else {
            return Vec::new();
        };
        let boundary = self
            .boundaries
            .iter()
            .copied()
            .find(|boundary| *boundary >= operation);
        self.discarding_until_sync |= boundary.is_none();
        let skipped =
            |id: &OperationId| *id >= operation && boundary.is_none_or(|boundary| *id <= boundary);
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
                metadata = MetadataSnapshot::default();
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
                later.metadata = metadata;
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
            if !retain_previous {
                metadata = MetadataSnapshot::default();
            }
            restored.push(RestoreMetadata {
                name,
                statement,
                metadata,
            });
        }
        restored
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
    pub(super) fn closed(&mut self, operation: OperationId) -> Option<MetadataSnapshot> {
        self.prepared_or_bound(operation)
    }
}

#[cfg(test)]
mod review_regressions {
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
        assert!(protocol.bind(
            original_bind,
            portal.clone(),
            &name,
            settings.clone(),
            false,
            Default::default()
        ));
        protocol.prepared_or_bound(original_bind);
        let rejected = operation_id();
        assert!(protocol.bind(
            rejected,
            portal.clone(),
            &name,
            settings.clone(),
            false,
            Default::default()
        ));
        protocol.sync(operation_id());
        let later = operation_id();
        assert!(protocol.bind(
            later,
            portal.clone(),
            &name,
            settings,
            false,
            Default::default()
        ));
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
        protocol.bind(
            old,
            portal.clone(),
            &name,
            settings.clone(),
            false,
            Default::default(),
        );
        protocol.prepared_or_bound(old);
        let sync = operation_id();
        protocol.sync(sync);
        let later = operation_id();
        protocol.bind(
            later,
            portal.clone(),
            &name,
            settings,
            false,
            Default::default(),
        );
        protocol.ready(Some(sync), true);
        protocol.failed(Some(later));
        assert!(!protocol.portals.contains_key(&portal));
    }

    #[test]
    fn review_r06_close_parse_bind_can_recover_before_replies() {
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
        assert!(protocol.bind(
            operation_id(),
            Name::new(),
            &name,
            EffectiveSettings::new(&ConnectionSettings::default()),
            false,
            Default::default()
        ));
    }
    #[test]
    fn review_r06_transaction_end_expires_invalidated_portals() {
        let mut protocol = SettingsProtocol::default();
        let name = Name::from_static(b"statement");
        let portal = Name::from_static(b"portal");
        let settings = EffectiveSettings::new(&ConnectionSettings::default());
        prepare(&mut protocol, &name);
        let first = operation_id();
        assert!(protocol.bind(
            first,
            portal.clone(),
            &name,
            settings.clone(),
            false,
            Default::default()
        ));
        protocol.prepared_or_bound(first);
        let rejected = operation_id();
        assert!(protocol.bind(
            rejected,
            portal.clone(),
            &name,
            settings.clone(),
            false,
            Default::default()
        ));
        protocol.failed(Some(rejected));
        protocol.ready(Some(operation_id()), true);
        assert!(protocol.bind(
            operation_id(),
            portal.clone(),
            &name,
            settings.clone(),
            false,
            Default::default()
        ));
        assert_eq!(
            protocol.execute(operation_id(), &portal, &ConnectionSettings::default()),
            Ok(false)
        );
    }
    #[test]
    fn review_r09_plain_transaction_control_does_not_require_drain() {
        for sql in ["BEGIN", "SAVEPOINT s", "RELEASE SAVEPOINT s"] {
            let mut protocol = SettingsProtocol::default();
            let statements = SqlParser::parse_statements(sql).unwrap();
            assert!(
                !protocol.simple_query(operation_id(), &statements, &ConnectionSettings::default()),
                "{sql}"
            );
        }
    }
}
