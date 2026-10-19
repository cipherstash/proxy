use super::{Deallocation, SessionId, Statement};
use crate::postgresql::rewrite::Name;
use std::{collections::HashMap, sync::Arc};

/// What Proxy holds for one statement prepared on the connection.
#[derive(Debug)]
struct PreparedStatement<K> {
    /// Encryption metadata, or `None` for a statement passed through unchanged
    statement: Option<Arc<Statement>>,
    metrics_scope: Option<SessionId>,
    /// The Parse operation that prepared the statement
    parse: Option<K>,
    /// Set when the statement's SQL is a `DEALLOCATE` or `DISCARD ALL`
    deallocation: Option<Deallocation>,
}

impl<K> Default for PreparedStatement<K> {
    fn default() -> Self {
        Self {
            statement: None,
            metrics_scope: None,
            parse: None,
            deallocation: None,
        }
    }
}

/// The statements prepared on a connection, by name.
///
/// Everything Proxy keeps for a statement lives in one entry, so closing or deallocating a
/// statement releases all of it together.
#[derive(Debug)]
pub(super) struct PreparedStatements<K> {
    statements: HashMap<Name, PreparedStatement<K>>,
}

impl<K> Default for PreparedStatements<K> {
    fn default() -> Self {
        Self {
            statements: HashMap::new(),
        }
    }
}

impl<K> PreparedStatements<K>
where
    K: Copy + PartialOrd,
{
    pub fn statement(&self, name: &Name) -> Option<Arc<Statement>> {
        self.statements.get(name)?.statement.clone()
    }

    pub fn set_statement(&mut self, name: Name, statement: Arc<Statement>) {
        self.statements.entry(name).or_default().statement = Some(statement);
    }

    pub fn metrics_scope(&self, name: &Name) -> Option<SessionId> {
        self.statements.get(name)?.metrics_scope
    }

    pub fn set_metrics_scope(&mut self, name: Name, session_id: SessionId) {
        self.statements.entry(name).or_default().metrics_scope = Some(session_id);
    }

    pub fn parse(&self, name: &Name) -> Option<K> {
        self.statements.get(name)?.parse
    }

    pub fn set_parse(&mut self, name: Name, operation: K) {
        self.statements.entry(name).or_default().parse = Some(operation);
    }

    pub fn deallocation(&self, name: &Name) -> Option<Deallocation> {
        self.statements.get(name)?.deallocation.clone()
    }

    pub fn set_deallocation(&mut self, name: Name, deallocation: Deallocation) {
        self.statements.entry(name).or_default().deallocation = Some(deallocation);
    }

    /// Remove the statement, returning its metrics scope.
    pub fn remove(&mut self, name: &Name) -> Option<SessionId> {
        self.statements.remove(name)?.metrics_scope
    }

    /// Keep only the statements for which `keep` returns true.
    pub fn retain(&mut self, mut keep: impl FnMut(&Name) -> bool) {
        self.statements.retain(|name, _| keep(name));
    }

    /// The names a deallocation applies to: the named statement, or every named statement
    /// Proxy holds. The unnamed statement is not removed by `DEALLOCATE ALL` or `DISCARD ALL`.
    pub fn deallocation_names(&self, deallocation: Deallocation) -> Vec<Name> {
        match deallocation {
            Deallocation::Statement(name) => vec![name],
            Deallocation::AllStatements => self
                .statements
                .keys()
                .filter(|name| !name.is_empty())
                .cloned()
                .collect(),
        }
    }

    /// Remove what a deallocation run by `operation` removed in PostgreSQL, returning the
    /// metrics scopes of the removed statements.
    ///
    /// Only statements prepared by a Parse before `operation` are removed. A statement with no
    /// recorded Parse is kept: leaking it is safe, while removing a statement PostgreSQL still
    /// has would let its next Bind through unencrypted.
    pub fn deallocate(&mut self, names: &[Name], operation: K) -> Vec<SessionId> {
        let mut scopes = vec![];
        for name in names {
            let prepared_earlier = self
                .statements
                .get(name)
                .and_then(|prepared| prepared.parse)
                .is_some_and(|parse| parse < operation);
            if prepared_earlier {
                scopes.extend(self.remove(name));
            }
        }
        scopes
    }

    pub fn references_metrics(&self, session_id: SessionId) -> bool {
        self.statements
            .values()
            .any(|prepared| prepared.metrics_scope == Some(session_id))
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.statements.len()
    }

    /// Names of the statements with a recorded Parse.
    #[cfg(test)]
    pub fn parsed_names(&self) -> Vec<String> {
        self.statements
            .iter()
            .filter(|(_, prepared)| prepared.parse.is_some())
            .map(|(name, _)| String::from_utf8_lossy(name).into_owned())
            .collect()
    }
}
