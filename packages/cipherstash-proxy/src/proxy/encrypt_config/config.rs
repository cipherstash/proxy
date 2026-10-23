use cipherstash_client::eql;
use cipherstash_client::schema::ColumnConfig;
use std::collections::HashMap;

///
/// Column configuration keyed by table name and column name
///    - key: `{table_name}.{column_name}`
///
type EncryptConfigMap = HashMap<eql::Identifier, ColumnConfig>;

#[derive(Clone, Debug, PartialEq)]
/// Encryption policies indexed by their resolved table and column names.
pub struct EncryptConfig {
    config: EncryptConfigMap,
}

impl EncryptConfig {
    /// Constructs encryption metadata from an already indexed configuration map.
    pub fn new_from_config(config: EncryptConfigMap) -> Self {
        Self { config }
    }

    /// Constructs an empty encryption configuration.
    pub fn new() -> Self {
        Self {
            config: HashMap::new(),
        }
    }

    /// Returns whether the snapshot contains no encrypted columns.
    pub fn is_empty(&self) -> bool {
        self.config.is_empty()
    }

    /// Returns the encryption policy for one resolved column.
    pub fn get_column_config(&self, identifier: &eql::Identifier) -> Option<ColumnConfig> {
        self.config.get(identifier).cloned()
    }

    /// Returns whether any encrypted column belongs to `table`.
    pub(crate) fn contains_table(&self, table: &str) -> bool {
        self.config
            .keys()
            .any(|identifier| identifier.table == table)
    }

    /// Inserts or replaces encryption metadata for one column.
    pub(crate) fn insert(&mut self, identifier: eql::Identifier, config: ColumnConfig) {
        self.config.insert(identifier, config);
    }

    /// Removes encryption metadata for one column.
    pub(crate) fn remove_column(&mut self, table: &str, column: &str) {
        self.config
            .remove(&eql::Identifier::new(table.to_owned(), column.to_owned()));
    }

    /// Removes all encryption metadata for a table.
    pub(crate) fn remove_table(&mut self, table: &str) {
        self.config
            .retain(|identifier, _| identifier.table != table);
    }

    /// Moves encryption metadata to a renamed column identifier.
    pub(crate) fn rename_column(&mut self, table: &str, from: &str, to: &str) {
        let from = eql::Identifier::new(table.to_owned(), from.to_owned());
        if let Some(config) = self.config.remove(&from) {
            self.config.insert(
                eql::Identifier::new(table.to_owned(), to.to_owned()),
                config,
            );
        }
    }

    /// Moves all encryption metadata to a renamed table identifier.
    pub(crate) fn rename_table(&mut self, from: &str, to: &str) {
        let renamed = self
            .config
            .iter()
            .filter(|(identifier, _)| identifier.table == from)
            .map(|(identifier, config)| (identifier.column.clone(), config.clone()))
            .collect::<Vec<_>>();
        self.remove_table(from);
        for (column, config) in renamed {
            self.config
                .insert(eql::Identifier::new(to.to_owned(), column), config);
        }
    }
}

impl Default for EncryptConfig {
    fn default() -> Self {
        Self::new()
    }
}
