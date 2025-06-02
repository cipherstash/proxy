use std::sync::{Arc, RwLock};

use sqltk::parser::ast::Ident;

use super::{Schema, SchemaError, SchemaTableColumn, SchemaWithEdits, Table};

#[derive(Debug)]
pub enum TableResolver {
    ViaSchema(Arc<Schema>),
    ViaSchemaWithEdits(Arc<RwLock<SchemaWithEdits>>),
}

impl TableResolver {
    pub fn new_fixed(schema: Arc<Schema>) -> Self {
        Self::ViaSchema(schema)
    }

    pub fn new_editable(schema: Arc<Schema>) -> Self {
        Self::ViaSchemaWithEdits(Arc::new(RwLock::new(SchemaWithEdits::new(schema))))
    }

    pub fn has_schema_changed(&self) -> bool {
        match self {
            TableResolver::ViaSchema(_) => false,
            TableResolver::ViaSchemaWithEdits(schema_with_edits) => {
                schema_with_edits.read().unwrap().has_schema_changed()
            }
        }
    }

    pub fn as_schema_with_edits(&self) -> Option<Arc<RwLock<SchemaWithEdits>>> {
        match self {
            TableResolver::ViaSchema(_) => None,
            TableResolver::ViaSchemaWithEdits(schema_with_edits) => Some(schema_with_edits.clone()),
        }
    }

    pub fn resolve_table(
        &self,
        table_name: &Ident,
        source_schema: &Ident,
    ) -> Result<Arc<Table>, SchemaError> {
        match self {
            TableResolver::ViaSchema(schema) => schema.resolve_table(table_name, source_schema),
            TableResolver::ViaSchemaWithEdits(schema_with_edits) => schema_with_edits
                .read()
                .unwrap()
                .resolve_table(table_name, source_schema),
        }
    }

    pub fn resolve_table_columns(
        &self,
        table_name: &Ident,
        source_schema: &Ident,
    ) -> Result<Vec<SchemaTableColumn>, SchemaError> {
        match self {
            TableResolver::ViaSchema(schema) => {
                schema.resolve_table_columns(table_name, source_schema)
            }
            TableResolver::ViaSchemaWithEdits(schema_with_edits) => schema_with_edits
                .read()
                .unwrap()
                .resolve_table_columns(table_name, source_schema),
        }
    }

    pub fn resolve_table_column(
        &self,
        table_name: &Ident,
        source_schema: &Ident,
        column_name: &Ident,
    ) -> Result<SchemaTableColumn, SchemaError> {
        match self {
            TableResolver::ViaSchema(schema) => {
                schema.resolve_table_column(table_name, source_schema, column_name)
            }
            TableResolver::ViaSchemaWithEdits(schema_with_edits) => schema_with_edits
                .read()
                .unwrap()
                .resolve_table_column(table_name, source_schema, column_name),
        }
    }
}
