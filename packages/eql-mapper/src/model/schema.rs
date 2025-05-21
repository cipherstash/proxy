//! Types that model a database schema.
//!
//! Column type information is unused currently.

use super::sql_ident::*;
use crate::iterator_ext::IteratorExt;
use core::fmt::Debug;
use derive_more::Display;
use sqltk::parser::ast::Ident;
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
    Eql(EqlTraitImpls),
}

/// The "traits" implemented by an EQL column which are derived from the index config for the column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct EqlTraitImpls {
    /// The EQL column implements equality between its values using the `=` operator.
    eq: bool,

    /// The EQL column implements comparison of its values using `>`, `>=`, `=`, `<=`, `<`.
    /// `ord` implies `eq`.
    ord: bool,

    /// The EQL column implements textual substring search using `LIKE`.
    bloom: bool,

    /// The EQL column implements a subset of the SQL JSON API (querying only).
    json: bool,
}

impl EqlTraitImpls {
    pub fn new(eq: bool, ord: bool, bloom: bool, json: bool) -> Self {
        Self { eq, ord, bloom, json }
    }

    pub fn implements_eq(&self) -> bool {
        self.eq || self.ord
    }

    pub fn implements_ord(&self) -> bool {
        self.ord
    }

    pub fn implements_bloom(&self) -> bool {
        self.bloom
    }

    pub fn implements_json(&self) -> bool {
        self.json
    }

    #[cfg(test)]
    pub(crate) fn impl_eq(&mut self) {
        self.eq = true;
    }

    #[cfg(test)]
    // Ord implies Eq.
    pub(crate) fn impl_ord(&mut self) {
        self.eq = true;
        self.ord = true;
    }

    #[cfg(test)]
    pub(crate) fn impl_bloom(&mut self) {
        self.bloom = true;
    }

    #[cfg(test)]
    pub(crate) fn impl_json(&mut self) {
        self.json = true;
    }
}

impl EqlTraitImpls {
    pub fn with<F>(f: F) -> Self where F: Fn(&mut Self) {
        let mut new = Self::default();
        f(&mut new);
        new
    }
}

impl Display for EqlTraitImpls {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut indexes: Vec<String> = vec![];

        if self.eq {
            indexes.push("Eq".into());
        }

        if self.ord {
            indexes.push("Ord".into());
        }

        if self.bloom {
            indexes.push("Bloom".into());
        }

        if self.json {
            indexes.push("Json".into());
        }

        f.write_str(&indexes.join("+"))?;

        Ok(())
    }
}

impl Column {
    pub fn eql(name: Ident, features: EqlTraitImpls) -> Self {
        Self {
            name,
            kind: ColumnKind::Eql(features),
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
            columns: Vec::with_capacity(16),
        }
    }

    /// Adds a column to the table.
    pub fn add_column(&mut self, column: Arc<Column>) -> Arc<Column> {
        self.columns.push(column);
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
}

#[macro_export]
macro_rules! to_eql_trait_impls {
    ($($indexes:ident)*) => {
        {
            #[allow(unused_mut)]
            let mut impls = $crate::EqlTraitImpls::default();
            $crate::to_eql_trait_impls!(@flags impls $($indexes)*);
            impls
        }
    };

    (@flags $impls:ident Eq $($indexes:ident)*) => {
        $impls.impl_eq();
        $crate::to_eql_trait_impls!(@flags $impls $($indexes)*);
    };

    (@flags $impls:ident Ord $($indexes:ident)*) => {
        $impls.impl_ord();
        $crate::to_eql_trait_impls!(@flags $impls $($indexes)*);
    };

    (@flags $impls:ident Bloom $($indexes:ident)*) => {
        $impls.impl_bloom();
        $crate::to_eql_trait_impls!(@flags $impls $($indexes)*);
    };

    (@flags $impls:ident Json $($indexes:ident)*) => {
        $impls.impl_json();
        $crate::to_eql_trait_impls!(@flags $impls $($indexes)*);
    };

    (@flags $impls:ident) => {}
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

                let mut $table = $crate::model::Table::new(::sqltk::parser::ast::Ident::new(stringify!($table_name)));
                schema!(@add_columns $table $($columns)*);
                $table
            }
        );
    };
    (@add_columns $table:ident $( $column_name:ident $(($($options:tt)+))? , )* ) => {
        $( schema!(@add_column $table $column_name $(($($options)*))? ); )*
    };
    (@add_column $table:ident $column_name:ident (EQL $(: $trait_:ident $(+ $trait_rest:ident)*)?) ) => {
        $table.add_column(std::sync::Arc::new($crate::model::Column::eql(
            ::sqltk::parser::ast::Ident::new(stringify!($column_name)),
            $crate::to_eql_trait_impls!($($trait_ $($($trait_rest)+)?)?)
        )), false);
    };
    (@add_column $table:ident $column_name:ident (PK) ) => {
        $table.add_column(
            std::sync::Arc::new(
                $crate::model::Column::native(
                    ::sqltk::parser::ast::Ident::new(stringify!($column_name))
                )
            ),
            true
        );
    };
    (@add_column $table:ident $column_name:ident () ) => {
        $table.add_column(
            std::sync::Arc::new(
                $crate::model::Column::new(
                    ::sqltk::parser::ast::Ident::new(stringify!($column_name)),
                    $crate::constraints::Scalar::Native {
                        table: $table.name.clone(),
                        column: ::sqltk::parser::ast::Ident::new(stringify!($column_name))
                    }
                )
            )
        );
    };
    (@add_column $table:ident $column_name:ident ) => {
        $table.add_column(
            std::sync::Arc::new(
                $crate::model::Column::native(
                    ::sqltk::parser::ast::Ident::new(stringify!($column_name)),
                )
            )
        );
    };
    // Main macro entry points
    {
        name: $schema_name:ident
        $(,$($rest:tt)*)?
    } => {
        {
            let schema_name = stringify!($schema_name);
            #[allow(unused_mut)]
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
            #[allow(unused_mut)]
            let mut schema = schema!(@schema schema schema_name);
            $( schema!(@match_tables schema $($rest)* ); )?
            schema
        }
    };
    { $($rest:tt)+ } => {
        {
            let schema_name = "public";
            #[allow(unused_mut)]
            let mut schema = schema!(@schema schema schema_name);
            schema!(@match_tables schema $($rest)* );
            schema
        }
    };
}
