use std::sync::{Arc, RwLock};

use sqltk::parser::ast::{Ident, ObjectName, ObjectNamePart};

use super::{IdentCase, Schema, SchemaError, SchemaTableColumn, SchemaWithEdits, Table};

/// The schema PostgreSQL keeps a connection's temporary tables in.
const TEMPORARY_SCHEMA: &str = "pg_temp";

#[derive(Debug)]
pub enum TableResolver {
    ViaSchema(Arc<Schema>),
    ViaSchemaWithEdits(Arc<RwLock<SchemaWithEdits>>),
    /// A connection's temporary tables in front of its permanent ones.
    WithTemporary {
        temporary: Arc<TableResolver>,
        permanent: Arc<TableResolver>,
    },
}

impl TableResolver {
    pub fn new_fixed(schema: Arc<Schema>) -> Self {
        Self::ViaSchema(schema)
    }

    pub fn new_editable(schema: Arc<Schema>) -> Self {
        Self::ViaSchemaWithEdits(Arc::new(RwLock::new(SchemaWithEdits::new(schema))))
    }

    /// An editable resolver for a connection's temporary tables, which DDL
    /// fills in. A table in it resolves by its bare name or as `pg_temp.name`.
    pub fn new_temporary() -> Self {
        Self::new_editable(Arc::new(Schema::new(TEMPORARY_SCHEMA)))
    }

    /// Resolves through `temporary` before `permanent`, as PostgreSQL searches
    /// `pg_temp` first.
    ///
    /// A bare name that both resolve does not resolve. PostgreSQL would pick
    /// the temporary table, but if Proxy's record of it is stale the name means
    /// the permanent table, so neither guess is safe.
    pub fn with_temporary(temporary: Arc<TableResolver>, permanent: Arc<TableResolver>) -> Self {
        Self::WithTemporary {
            temporary,
            permanent,
        }
    }

    pub fn has_schema_changed(&self) -> bool {
        match self {
            TableResolver::ViaSchema(_) => false,
            TableResolver::ViaSchemaWithEdits(schema_with_edits) => {
                schema_with_edits.read().unwrap().has_schema_changed()
            }
            TableResolver::WithTemporary { permanent, .. } => permanent.has_schema_changed(),
        }
    }

    pub fn as_schema_with_edits(&self) -> Option<Arc<RwLock<SchemaWithEdits>>> {
        match self {
            TableResolver::ViaSchema(_) => None,
            TableResolver::ViaSchemaWithEdits(schema_with_edits) => Some(schema_with_edits.clone()),
            TableResolver::WithTemporary { permanent, .. } => permanent.as_schema_with_edits(),
        }
    }

    /// The resolver that owns `name`: this one, or for [`Self::WithTemporary`]
    /// the layer the name resolves in.
    fn owner(&self, name: &ObjectName) -> Result<&TableResolver, SchemaError> {
        let TableResolver::WithTemporary {
            temporary,
            permanent,
        } = self
        else {
            return Ok(self);
        };

        match name.0.as_slice() {
            [ObjectNamePart::Identifier(qualifier), _]
                if IdentCase(qualifier) == IdentCase(&Ident::new(TEMPORARY_SCHEMA)) =>
            {
                temporary.owner(name)
            }
            [_] => match (
                temporary.resolve_table(name).is_ok(),
                permanent.resolve_table(name).is_ok(),
            ) {
                (true, true) => Err(SchemaError::AmbiguousTable(name.to_string())),
                (true, false) => temporary.owner(name),
                (false, _) => permanent.owner(name),
            },
            _ => permanent.owner(name),
        }
    }

    pub fn resolve_table(&self, name: &ObjectName) -> Result<Arc<Table>, SchemaError> {
        match self.owner(name)? {
            TableResolver::ViaSchema(schema) => schema.resolve_table(name),
            TableResolver::ViaSchemaWithEdits(schema_with_edits) => {
                schema_with_edits.read().unwrap().resolve_table(name)
            }
            TableResolver::WithTemporary { .. } => unreachable!("owner is a single layer"),
        }
    }

    pub fn resolve_table_columns(
        &self,
        table_name: &ObjectName,
    ) -> Result<Vec<SchemaTableColumn>, SchemaError> {
        match self.owner(table_name)? {
            TableResolver::ViaSchema(schema) => schema.resolve_table_columns(table_name),
            TableResolver::ViaSchemaWithEdits(schema_with_edits) => schema_with_edits
                .read()
                .unwrap()
                .resolve_table_columns(table_name),
            TableResolver::WithTemporary { .. } => unreachable!("owner is a single layer"),
        }
    }

    pub fn resolve_table_column(
        &self,
        table_name: &ObjectName,
        column_name: &Ident,
    ) -> Result<SchemaTableColumn, SchemaError> {
        match self.owner(table_name)? {
            TableResolver::ViaSchema(schema) => {
                schema.resolve_table_column(table_name, column_name)
            }
            TableResolver::ViaSchemaWithEdits(schema_with_edits) => schema_with_edits
                .read()
                .unwrap()
                .resolve_table_column(table_name, column_name),
            TableResolver::WithTemporary { .. } => unreachable!("owner is a single layer"),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use sqltk::parser::ast::{Ident, ObjectName, ObjectNamePart};

    use crate::{
        collect_ddl, schema,
        test_helpers::{id, object_name, parse},
        SchemaError, TableResolver,
    };

    fn qualified(schema_name: &str, table_name: &str) -> ObjectName {
        ObjectName(vec![
            ObjectNamePart::Identifier(Ident::new(schema_name)),
            ObjectNamePart::Identifier(Ident::new(table_name)),
        ])
    }

    /// A resolver over `users (id, email EQL)` with temporary tables created by `ddl`.
    fn resolver(ddl: &str) -> TableResolver {
        let permanent = Arc::new(TableResolver::new_editable(Arc::new(schema! {
            tables: {
                users: {
                    id,
                    email (EQL),
                }
            }
        })));
        let temporary = Arc::new(TableResolver::new_temporary());
        collect_ddl(temporary.clone(), &parse(ddl));
        TableResolver::with_temporary(temporary, permanent)
    }

    #[test]
    fn a_bare_name_resolves_to_a_temporary_table() {
        let resolver = resolver("create temporary table names (name text)");

        assert!(resolver
            .resolve_table_column(&object_name("names"), &id("name"))
            .is_ok());
    }

    #[test]
    fn only_a_pg_temp_qualifier_resolves_to_a_temporary_table() {
        let resolver = resolver("create temporary table names (name text)");

        assert!(resolver
            .resolve_table(&qualified("pg_temp", "names"))
            .is_ok());
        assert_eq!(
            resolver.resolve_table(&qualified("public", "names")),
            Err(SchemaError::TableNotFound("public.names".into()))
        );
    }

    #[test]
    fn permanent_tables_resolve_as_before() {
        let resolver = resolver("create temporary table names (name text)");

        assert!(resolver.resolve_table(&object_name("users")).is_ok());
        assert!(resolver
            .resolve_table(&qualified("public", "users"))
            .is_ok());
        assert!(resolver
            .resolve_table(&qualified("pg_temp", "users"))
            .is_err());
    }

    /// PostgreSQL would resolve the bare name to the temporary table, but a
    /// temporary table Proxy thinks exists may already be gone, and then the
    /// name means the permanent table. Neither guess is safe.
    #[test]
    fn a_temporary_table_that_shadows_a_permanent_one_does_not_resolve() {
        let resolver = resolver("create temporary table users (id int)");

        assert_eq!(
            resolver.resolve_table(&object_name("users")),
            Err(SchemaError::AmbiguousTable("users".into()))
        );
        assert!(resolver
            .resolve_table(&qualified("pg_temp", "users"))
            .is_ok());
        assert!(resolver
            .resolve_table(&qualified("public", "users"))
            .is_ok());
    }

    #[test]
    fn a_dropped_temporary_table_does_not_resolve() {
        let resolver = resolver("create temporary table names (name text)");
        let TableResolver::WithTemporary { temporary, .. } = &resolver else {
            unreachable!()
        };
        collect_ddl(temporary.clone(), &parse("drop table names"));

        assert!(resolver.resolve_table(&object_name("names")).is_err());
    }
}
