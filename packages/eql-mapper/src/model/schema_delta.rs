use std::{
    collections::HashMap,
    convert::Infallible,
    ops::ControlFlow,
    sync::{Arc, RwLock},
};

use sqltk::parser::ast::{
    AlterTableOperation, ColumnDef, CreateTable, Ident, ObjectName, ObjectType, Statement,
    ViewColumnDef,
};

use sqltk::{Break, Visitable, Visitor};

use super::{
    Column, ColumnKind, Schema, SchemaError, SchemaTableColumn, SqlIdent, Table, TableResolver,
};

/// The current state of the schema as viewed by the current transaction.
///
/// When used in conjunction with the [`crate`], DDL statements will be parsed and an "overlay" schema
/// captures differences from the schema that was already loaded.
///
/// All table and column lookups during EQL mapping will go through via the overlay scheme, falling back to the
/// loaded schema.
#[derive(Debug)]
pub struct SchemaWithEdits {
    schema: Arc<Schema>,
    overlays: HashMap<SqlIdent<Ident>, Overlay>,
}

impl SchemaWithEdits {
    pub fn new(schema: Arc<Schema>) -> Self {
        Self {
            schema,
            overlays: HashMap::new(),
        }
    }

    pub fn has_schema_changed(&self) -> bool {
        !self.overlays.is_empty()
    }

    /// Gets or creates a [`TableOverlay`] for a table named `table_name`.
    ///
    /// If there is no existing overlay for the table, then a new overlay will be created using
    /// `TableOverlay::Table(_)` where the table is copied from the [`Schema`].
    fn get_overlay_mut(&mut self, table_name: &Ident) -> &mut Overlay {
        let overlay = {
            let schema_table = self.schema.resolve_table(table_name);
            match schema_table {
                Ok(schema_table) => Overlay::Table(OverlayTable::from(&*schema_table)),
                Err(_) => Overlay::Table(OverlayTable::new(table_name.clone())),
            }
        };

        self.overlays
            .entry(SqlIdent(table_name.clone()))
            .or_insert(overlay)
    }

    pub(crate) fn resolve_table(&self, name: &Ident) -> Result<Arc<Table>, SchemaError> {
        match self.overlays.get(&SqlIdent(name.clone())) {
            Some(overlay) => match overlay {
                Overlay::Dropped => Err(SchemaError::TableNotFound(name.to_string())),
                Overlay::Table(overlay_table) => Ok(Arc::new(overlay_table.into())),
            },
            None => self.schema.resolve_table(name),
        }
    }

    pub(crate) fn resolve_table_columns(
        &self,
        table_name: &Ident,
    ) -> Result<Vec<SchemaTableColumn>, SchemaError> {
        let table = self.resolve_table(table_name)?;
        Ok(table
            .columns
            .iter()
            .map(|col| SchemaTableColumn {
                table: table.name.clone(),
                column: col.name.clone(),
                kind: col.kind,
            })
            .collect())
    }

    pub(crate) fn resolve_table_column(
        &self,
        table_name: &Ident,
        column_name: &Ident,
    ) -> Result<SchemaTableColumn, SchemaError> {
        let table = self.resolve_table(table_name)?;
        match table
            .columns
            .iter()
            .find(|col| SqlIdent(&col.name) == SqlIdent(column_name))
        {
            Some(col) => Ok(SchemaTableColumn {
                table: table_name.clone(),
                column: column_name.clone(),
                kind: col.kind,
            }),
            None => Err(SchemaError::ColumnNotFound(
                table_name.to_string(),
                column_name.to_string(),
            )),
        }
    }
}

/// Acts like a mask over a table or an existing table that has been dropped in the current transaction.
#[derive(Debug)]
enum Overlay {
    /// Hides the existence of table in the main [`Schema`] causing resolution of that table to fail.
    Dropped,

    /// A newly added table/view or modification to an existing table.
    Table(OverlayTable),
}

/// A mutable version of [`Table`].
#[derive(Debug, Clone)]
struct OverlayTable {
    pub name: Ident,
    pub columns: Vec<Column>,
    pub primary_key: Vec<usize>,
}

impl OverlayTable {
    fn new(name: Ident) -> Self {
        Self {
            name,
            columns: Vec::new(),
            primary_key: Vec::new(),
        }
    }

    fn add_column(&mut self, col: Column) {
        self.columns.push(col);
    }

    fn remove_column(&mut self, name: &Ident) {
        if let Some((idx, _)) = self
            .columns
            .iter()
            .enumerate()
            .find(|(_, col)| col.name == *name)
        {
            self.primary_key.retain(|col| *col != idx);
            self.columns.retain(|col| col.name != *name);
        }
    }

    fn rename_column(&mut self, old_column_name: &Ident, new_column_name: &Ident) {
        if let Some(col) = self
            .columns
            .iter_mut()
            .find(|col| col.name == *old_column_name)
        {
            col.name = new_column_name.clone();
        }
    }

    fn rename(&mut self, to: Ident) {
        self.name = to;
    }
}

impl From<&Table> for OverlayTable {
    fn from(value: &Table) -> Self {
        Self {
            name: value.name.clone(),
            columns: value.columns.iter().map(|col| (**col).clone()).collect(),
            primary_key: value.primary_key.clone(),
        }
    }
}

impl From<&OverlayTable> for Table {
    fn from(value: &OverlayTable) -> Self {
        Self {
            name: value.name.clone(),
            columns: value.columns.iter().cloned().map(Arc::new).collect(),
            primary_key: value.primary_key.clone(),
        }
    }
}

/// Applies any DDL found in `statement` to `table_resolver` if `table_resolver` is a `TableResolver::ViaSchemaWithEdits(_)`.
///
/// Returns `true` if `statement` contained relevant DDL (regardless of `TableResolver` variant).
pub fn collect_ddl(table_resolver: Arc<TableResolver>, statement: &Statement) -> bool {
    if let Some(schema_with_edits) = table_resolver.as_schema_with_edits() {
        let mut visitor = DdlCollector {
            schema: schema_with_edits,
            changed: false,
        };
        statement.accept(&mut visitor);
        return visitor.changed;
    }

    table_resolver.has_schema_changed()
}

struct DdlCollector {
    schema: Arc<RwLock<SchemaWithEdits>>,
    changed: bool,
}

impl DdlCollector {
    fn capture_create_view(&self, name: &ObjectName, columns: &[ViewColumnDef]) {
        let name = name.0.last().unwrap().clone();
        let mut table = OverlayTable::new(name.clone());

        for def in columns {
            table.add_column(Column {
                name: def.name.clone(),
                kind: ColumnKind::Native,
            });
        }

        *self.schema.write().unwrap().get_overlay_mut(&name) = Overlay::Table(table)
    }

    fn capture_create_table(&self, name: &ObjectName, columns: &[ColumnDef]) {
        let name = name.0.last().unwrap().clone();
        let mut table = OverlayTable::new(name.clone());

        for def in columns {
            table.add_column(Column {
                name: def.name.clone(),
                kind: ColumnKind::Native,
            });
        }

        *self.schema.write().unwrap().get_overlay_mut(&name) = Overlay::Table(table)
    }

    fn capture_alter_table(&self, name: &ObjectName, operations: &[AlterTableOperation]) {
        let table_name = name.0.last().unwrap();

        for op in operations {
            match op {
                AlterTableOperation::AddColumn { column_def, .. } => {
                    let mut overlay_schema = self.schema.write().unwrap();
                    let overlay = overlay_schema.get_overlay_mut(table_name);
                    if let Overlay::Table(table) = overlay {
                        table.add_column(Column {
                            name: column_def.name.clone(),
                            kind: ColumnKind::Native,
                        });
                    }
                }

                AlterTableOperation::DropColumn {
                    column_name,
                    cascade: false,
                    ..
                } => {
                    let mut overlay_schema = self.schema.write().unwrap();
                    let overlay = overlay_schema.get_overlay_mut(table_name);
                    if let Overlay::Table(table) = overlay {
                        table.remove_column(column_name);
                    }
                }

                AlterTableOperation::RenameColumn {
                    old_column_name,
                    new_column_name,
                } => {
                    let mut overlay_schema = self.schema.write().unwrap();
                    let overlay = overlay_schema.get_overlay_mut(table_name);
                    if let Overlay::Table(table) = overlay {
                        table.rename_column(old_column_name, new_column_name);
                    }
                }

                AlterTableOperation::RenameTable { table_name: to } => {
                    let mut overlay_schema = self.schema.write().unwrap();
                    let overlay = overlay_schema.get_overlay_mut(table_name);
                    let new_name = to.0.last().unwrap().clone();

                    let table = match overlay {
                        Overlay::Table(table) => Some(table.clone()),
                        _ => None,
                    };

                    if let Some(mut table_to_rename) = table {
                        // Mark old table name as dropped so it no longer resolves
                        *overlay = Overlay::Dropped;
                        table_to_rename.rename(new_name.clone());
                        // Appease the borrow checker: relinquish the borrow, then reborrow.
                        drop(overlay_schema);
                        let mut overlay_schema = self.schema.write().unwrap();
                        let overlay = overlay_schema.get_overlay_mut(&new_name);
                        // Insert table with new name.
                        *overlay = Overlay::Table(table_to_rename);
                    }
                }

                AlterTableOperation::ChangeColumn {
                    old_name, new_name, ..
                } => {
                    let mut overlay_schema = self.schema.write().unwrap();
                    let overlay = overlay_schema.get_overlay_mut(table_name);
                    if let Overlay::Table(table) = overlay {
                        table.rename_column(old_name, new_name);
                    }
                }

                _ => {}
            }
        }
    }

    fn capture_drop_tables(&self, names: &[ObjectName]) {
        let mut overlay_schema = self.schema.write().unwrap();

        for name in names {
            let overlay = overlay_schema.get_overlay_mut(&name.0.last().unwrap().clone());
            *overlay = Overlay::Dropped;
        }
    }
}

impl<'ast> Visitor<'ast> for DdlCollector {
    type Error = Infallible;

    fn enter<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        if let Some(statement) = node.downcast_ref::<Statement>() {
            match statement {
                Statement::CreateView { name, columns, .. } => {
                    self.capture_create_view(name, columns);
                    self.changed = true;
                }

                Statement::CreateTable(CreateTable { name, columns, .. }) => {
                    self.capture_create_table(name, columns);
                    self.changed = true;
                }

                Statement::AlterTable {
                    name, operations, ..
                } => {
                    self.capture_alter_table(name, operations);
                    self.changed = true;
                }

                Statement::Drop {
                    object_type: ObjectType::Table | ObjectType::View,
                    cascade: false,
                    names,
                    ..
                } => {
                    self.capture_drop_tables(names);
                    self.changed = true;
                }

                _ => {}
            }
        }

        ControlFlow::Continue(())
    }
}

#[cfg(test)]
mod test {
    use std::sync::Arc;

    use crate::{
        schema,
        test_helpers::{id, parse},
        ColumnKind, SchemaError, SchemaTableColumn, TableResolver,
    };

    #[test]
    fn add_column() {
        let schema = Arc::new(schema! {
            tables: {
                users: {
                    id (PK),
                    email,
                }
            }
        });

        let resolver = Arc::new(TableResolver::new_editable(schema));

        let statement = parse("alter table users add age int");

        crate::collect_ddl(resolver.clone(), &statement);

        assert_eq!(
            resolver.resolve_table_column(&id("users"), &id("age")),
            Ok(SchemaTableColumn {
                table: id("users"),
                column: id("age"),
                kind: crate::ColumnKind::Native
            })
        )
    }

    #[test]
    fn drop_column() {
        let schema = Arc::new(schema! {
            tables: {
                users: {
                    id (PK),
                    email,
                }
            }
        });

        let resolver = Arc::new(TableResolver::new_editable(schema));

        let statement = parse("alter table users drop column email");

        crate::collect_ddl(resolver.clone(), &statement);

        assert_eq!(
            resolver.resolve_table_column(&id("users"), &id("email")),
            Err(SchemaError::ColumnNotFound("users".into(), "email".into()))
        )
    }

    #[test]
    fn rename_column() {
        let schema = Arc::new(schema! {
            tables: {
                users: {
                    id (PK),
                    email (EQL),
                }
            }
        });

        let resolver = Arc::new(TableResolver::new_editable(schema));

        let statement = parse("alter table users rename column email to primary_email");

        crate::collect_ddl(resolver.clone(), &statement);

        assert_eq!(
            resolver.resolve_table_column(&id("users"), &id("email")),
            Err(SchemaError::ColumnNotFound("users".into(), "email".into()))
        );

        assert_eq!(
            resolver.resolve_table_column(&id("users"), &id("primary_email")),
            Ok(SchemaTableColumn {
                table: id("users"),
                column: id("primary_email"),
                kind: ColumnKind::Eql
            })
        )
    }

    #[test]
    fn rename_table() {
        let schema = Arc::new(schema! {
            tables: {
                users: {
                    id (PK),
                    email (EQL),
                }
            }
        });

        let resolver = Arc::new(TableResolver::new_editable(schema));

        let statement = parse("alter table users rename to app_users");

        crate::collect_ddl(resolver.clone(), &statement);

        assert_eq!(
            resolver.resolve_table_column(&id("users"), &id("email")),
            Err(SchemaError::TableNotFound("users".into()))
        );

        assert_eq!(
            resolver.resolve_table_column(&id("app_users"), &id("email")),
            Ok(SchemaTableColumn {
                table: id("app_users"),
                column: id("email"),
                kind: ColumnKind::Eql
            })
        )
    }

    #[test]
    fn create_table() {
        let schema = Arc::new(schema! {
            tables: { }
        });

        let resolver = Arc::new(TableResolver::new_editable(schema));

        assert_eq!(
            resolver.resolve_table_column(&id("users"), &id("email")),
            Err(SchemaError::TableNotFound("users".into()))
        );

        let statement = parse("create table users (id serial, email text)");

        crate::collect_ddl(resolver.clone(), &statement);

        assert_eq!(
            resolver.resolve_table_column(&id("users"), &id("email")),
            Ok(SchemaTableColumn {
                table: id("users"),
                column: id("email"),
                kind: ColumnKind::Native
            })
        )
    }

    #[test]
    fn drop_table() {
        let schema = Arc::new(schema! {
            tables: {
                users: {
                    id (PK),
                    email (EQL),
                }
            }
        });

        let resolver = Arc::new(TableResolver::new_editable(schema));

        assert_eq!(
            resolver.resolve_table_column(&id("users"), &id("email")),
            Ok(SchemaTableColumn {
                table: id("users"),
                column: id("email"),
                kind: ColumnKind::Eql
            })
        );

        let statement = parse("drop table users");

        crate::collect_ddl(resolver.clone(), &statement);

        assert_eq!(
            resolver.resolve_table_column(&id("users"), &id("email")),
            Err(SchemaError::TableNotFound("users".into()))
        )
    }
}
