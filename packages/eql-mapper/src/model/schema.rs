//! Types that model a database schema.
//!
//! Column type information is unused currently.

use super::sql_ident::*;
use crate::iterator_ext::IteratorExt;
use core::fmt::Debug;
use derive_more::Display;
use sqlparser::ast::Ident;
use std::sync::Arc;
use thiserror::Error;

/// A database schema.
///
/// It has a name and some tables. Tables and views are represented identically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Schema {
    pub name: Ident,
    pub tables: Vec<Arc<Table>>,
    pub aggregates: Vec<Arc<String>>,
}

/// A table (or view).
///
/// It has a name and some columns
#[derive(Debug, Clone, PartialEq, Eq, Display, Hash)]
#[display("Table<{}>", name)]
pub struct Table {
    pub name: Ident,
    pub columns: Vec<Arc<Column>>,
    // Stores indices into the columns Vec.
    pub primary_key: Vec<usize>,
}

/// A column.
#[derive(Debug, Clone, PartialEq, Eq, Display, Hash)]
#[display("Column<{}: {}>", name, kind)]
pub struct Column {
    pub name: Ident,
    pub kind: ColumnKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Display, Hash)]
pub enum ColumnKind {
    Native,
    Eql,
}

impl Column {
    pub fn eql(name: Ident) -> Self {
        Self {
            name,
            kind: ColumnKind::Eql,
        }
    }

    pub fn native(name: Ident) -> Self {
        Self {
            name,
            kind: ColumnKind::Native,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Display)]
#[display("{}.{}", table, column)]
pub struct SchemaTableColumn {
    pub table: Ident,
    pub column: Ident,
    pub kind: ColumnKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum SchemaError {
    #[error("Table not found: {}", _0)]
    TableNotFound(String),

    #[error("Column: {} not found for table: {}", _0, _1)]
    ColumnNotFound(String, String),
}

impl Schema {
    /// Creates a new named empty schema.
    pub fn new<S>(name: S) -> Self
    where
        S: Into<String>,
    {
        let name = Ident::new(name);
        // name.quote_style = Some('"');

        Self {
            name,
            tables: Default::default(),
            aggregates: Default::default(),
        }
    }

    /// Adds a table (or view) to the schema.
    pub fn add_table(&mut self, table: Table) {
        self.tables.push(Arc::new(table));
    }

    /// Resolves a table by `Ident`, which takes into account the SQL rules
    /// of quoted and new identifier matching.
    pub fn resolve_table(&self, name: &Ident) -> Result<Arc<Table>, SchemaError> {
        let mut haystack = self.tables.iter();
        haystack
            .find_unique(&|table| SqlIdent::from(&table.name) == SqlIdent::from(name))
            .cloned()
            .map_err(|_| SchemaError::TableNotFound(name.to_string()))
    }

    pub fn resolve_table_columns(
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

    pub fn resolve_table_column(
        &self,
        table_name: &Ident,
        column_name: &Ident,
    ) -> Result<SchemaTableColumn, SchemaError> {
        let mut haystack = self.tables.iter();
        match haystack
            .find_unique(&|table| SqlIdent::from(&table.name) == SqlIdent::from(table_name))
        {
            Ok(table) => match table.get_column(column_name) {
                Ok(column) => Ok(SchemaTableColumn {
                    table: table.name.clone(),
                    column: column.name.clone(),
                    kind: column.kind,
                }),
                Err(_) => Err(SchemaError::ColumnNotFound(
                    table_name.to_string(),
                    column_name.to_string(),
                )),
            },
            Err(_) => Err(SchemaError::TableNotFound(table_name.to_string())),
        }
    }
}

impl Table {
    /// Create a new named table with no columns.
    pub fn new(name: Ident) -> Self {
        Self {
            name,
            primary_key: Vec::with_capacity(1),
            columns: Vec::with_capacity(16),
        }
    }

    /// Adds a column to the table.
    pub fn add_column(&mut self, column: Arc<Column>, part_of_primary_key: bool) -> Arc<Column> {
        self.columns.push(column);
        if part_of_primary_key {
            self.primary_key.push(self.columns.len() - 1);
        }
        self.columns[self.columns.len() - 1].clone()
    }

    /// Checks if a column named `name` exists in the table.
    pub fn contains_column(&self, name: &Ident) -> bool {
        self.get_column(name).is_ok()
    }

    /// Gets a column from a table by name.
    pub fn get_column(&self, name: &Ident) -> Result<Arc<Column>, SchemaError> {
        let mut haystack = self.columns.iter();
        haystack
            .find_unique(&|column| SqlIdent::from(&column.name) == SqlIdent::from(name))
            .cloned()
            .map_err(|_| SchemaError::ColumnNotFound(self.name.to_string(), name.to_string()))
    }

    /// Gets all the primary key columns in the table.
    pub fn get_primary_key_columns(&self) -> Vec<Arc<Column>> {
        self.primary_key
            .iter()
            .filter_map(|index| self.columns.get(*index))
            .cloned()
            .collect()
    }
}

/// A DSL to create a [`Schema`] for testing purposes.
// #[cfg(test)]
#[macro_export]
macro_rules! schema {
    (@name $schema_name:literal) => {
        stringify!($schema_name)
    };
    (@schema $schema:ident $schema_name:ident) => {
        $crate::model::Schema::new($schema_name)
    };
    (@schema $schema:ident) => {
        $crate::schema::Schema::new("public")
    };
    (
        @match_tables $schema:ident
        tables: {
            $($table_name:ident : $column_defs:tt)*
        }
        $(,$($rest:tt)*)?
    ) => {
        {
            $( schema!(@add_table $schema $table_name table $column_defs); )*
            $( schema!(@add_aggregates $schema $($rest)*); )?
        }
    };
    (@add_aggregates $schema:ident [ $($aggregate_name:ident),* ]) => {
        {
            $schema.aggregates = vec![$($aggregate_name,)* ];
        }
    };
    (@add_table $schema:ident $table_name:ident $table:ident { $($columns:tt)* }) => {
        $schema.add_table(
            {

                let mut $table = $crate::model::Table::new(::sqlparser::ast::Ident::new(stringify!($table_name)));
                schema!(@add_columns $table $($columns)*);
                $table
            }
        );
    };
    (@add_columns $table:ident $( $column_name:ident $(($($options:tt)+))? , )* ) => {
        $( schema!(@add_column $table $column_name $(($($options)*))? ); )*
    };
    (@add_column $table:ident $column_name:ident (EQL) ) => {
        $table.add_column(std::sync::Arc::new($crate::model::Column::eql(
            ::sqlparser::ast::Ident::new(stringify!($column_name))
        )), false);
    };
    (@add_column $table:ident $column_name:ident (PK) ) => {
        $table.add_column(
            std::sync::Arc::new(
                $crate::model::Column::native(
                    ::sqlparser::ast::Ident::new(stringify!($column_name))
                )
            ),
            true
        );
    };
    (@add_column $table:ident $column_name:ident () ) => {
        $table.add_column(
            std::sync::Arc::new(
                $crate::model::Column::new(
                    ::sqlparser::ast::Ident::new(stringify!($column_name)),
                    $crate::constraints::Scalar::Native {
                        table: $table.name.clone(),
                        column: ::sqlparser::ast::Ident::new(stringify!($column_name))
                    }
                )
            ),
            false
        );
    };
    (@add_column $table:ident $column_name:ident ) => {
        $table.add_column(
            std::sync::Arc::new(
                $crate::model::Column::native(
                    ::sqlparser::ast::Ident::new(stringify!($column_name)),
                )
            ),
            false
        );
    };
    // Main macro entry points
    {
        name: $schema_name:ident
        $(,$($rest:tt)*)?
    } => {
        {
            let schema_name = stringify!($schema_name);
            let mut schema = schema!(@schema schema schema_name);
            $( schema!(@match_tables schema $($rest)* ); )?
            schema
        }
    };
    {
        name: $schema_name:literal
        $(,$($rest:tt)*)?
    } => {
        {
            let schema_name = $schema_name;
            let schema = schema!(@schema schema schema_name);
            $( schema!(@match_tables schema $($rest)* ); )?
            schema
        }
    };
    { $($rest:tt)+ } => {
        {
            let schema_name = "public";
            let mut schema = schema!(@schema schema schema_name);
            schema!(@match_tables schema $($rest)* );
            schema
        }
    };
}
