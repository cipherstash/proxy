//! Types that model a database schema.
//!
//! Column type information is unused currently.

use super::ident_case::*;
use crate::{
    iterator_ext::IteratorExt,
    unifier::{DomainIdentity, EqlTraits},
};
use core::fmt::Debug;
use derive_more::Display;
use sqltk::parser::ast::{Ident, ObjectName, ObjectNamePart};
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

#[derive(Debug, Clone, PartialEq, Eq, Display, Hash)]
pub enum ColumnKind {
    #[display("Native")]
    Native,
    /// An encrypted column: its capabilities, plus the inert v3 domain identity
    /// (see ADR-0002), which the v3 schema loader populates from the Postgres
    /// domain name.
    #[display("Eql({})", _0)]
    Eql(EqlTraits, DomainIdentity),
    /// A column whose declared type says "this holds encrypted data", but which
    /// this build of Proxy cannot map. Today that is only the legacy EQL v2
    /// `eql_v2_encrypted` type, which carries no v3 domain identity.
    ///
    /// It is deliberately **not** [`ColumnKind::Native`]. A native column is a
    /// plaintext passthrough — no encryption on write, no decryption on read —
    /// so classifying an unmigrated v2 column that way makes Proxy quietly write
    /// new plaintext into a column its operator believes is encrypted
    /// (CIP-3688). Instead the type checker refuses any statement that brings
    /// the owning table into scope, so the failure is loud and names the column.
    ///
    /// The payload is the declared type name, purely so the refusal can quote it
    /// back to the operator.
    #[display("Unmappable({})", _0)]
    UnmappableEncrypted(String),
}

impl Column {
    pub fn eql(name: Ident, features: EqlTraits, identity: DomainIdentity) -> Self {
        Self {
            name,
            kind: ColumnKind::Eql(features, identity),
        }
    }

    pub fn native(name: Ident) -> Self {
        Self {
            name,
            kind: ColumnKind::Native,
        }
    }

    /// A column declared with an encrypted-column type this build cannot map.
    /// See [`ColumnKind::UnmappableEncrypted`].
    pub fn unmappable_encrypted(name: Ident, column_type: impl Into<String>) -> Self {
        Self {
            name,
            kind: ColumnKind::UnmappableEncrypted(column_type.into()),
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
    pub fn resolve_table(&self, name: &ObjectName) -> Result<Arc<Table>, SchemaError> {
        if name.0.len() == 1 {
            let ObjectNamePart::Identifier(name) = name.0.last().unwrap();
            let mut haystack = self.tables.iter();
            haystack
                .find_unique(&|table| IdentCase::from(&table.name) == IdentCase::from(name))
                .cloned()
                .map_err(|_| SchemaError::TableNotFound(name.to_string()))
        } else {
            Err(SchemaError::TableNotFound(format!("{name}")))
        }
    }

    pub fn resolve_table_columns(
        &self,
        table_name: &ObjectName,
    ) -> Result<Vec<SchemaTableColumn>, SchemaError> {
        let table = self.resolve_table(table_name)?;
        Ok(table
            .columns
            .iter()
            .map(|col| SchemaTableColumn {
                table: table.name.clone(),
                column: col.name.clone(),
                kind: col.kind.clone(),
            })
            .collect())
    }

    pub fn resolve_table_column(
        &self,
        table_name: &ObjectName,
        column_name: &Ident,
    ) -> Result<SchemaTableColumn, SchemaError> {
        if table_name.0.len() == 1 {
            let ObjectNamePart::Identifier(table_name) = table_name.0.last().unwrap();
            let mut haystack = self.tables.iter();
            match haystack
                .find_unique(&|table| IdentCase::from(&table.name) == IdentCase::from(table_name))
            {
                Ok(table) => match table.get_column(column_name) {
                    Ok(column) => Ok(SchemaTableColumn {
                        table: table.name.clone(),
                        column: column.name.clone(),
                        kind: column.kind.clone(),
                    }),
                    Err(_) => Err(SchemaError::ColumnNotFound(
                        table_name.to_string(),
                        column_name.to_string(),
                    )),
                },
                Err(_) => Err(SchemaError::TableNotFound(table_name.to_string())),
            }
        } else {
            Err(SchemaError::ColumnNotFound(
                format!("{table_name}"),
                format!("{column_name}"),
            ))
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
            .find_unique(&|column| IdentCase::from(&column.name) == IdentCase::from(name))
            .cloned()
            .map_err(|_| SchemaError::ColumnNotFound(self.name.to_string(), name.to_string()))
    }
}

#[macro_export]
macro_rules! to_eql_trait_impls {
    ($($indexes:ident)*) => {
        {
            #[allow(unused_mut)]
            let mut impls = $crate::unifier::EqlTraits::default();
            $crate::to_eql_trait_impls!(@flags impls $($indexes)*);
            impls
        }
    };

    (@flags $impls:ident Eq $($indexes:ident)*) => {
        $impls.add_mut(EqlTrait::Eq);
        $crate::to_eql_trait_impls!(@flags $impls $($indexes)*);
    };

    (@flags $impls:ident Ord $($indexes:ident)*) => {
        $impls.add_mut(EqlTrait::Ord);
        $crate::to_eql_trait_impls!(@flags $impls $($indexes)*);
    };

    (@flags $impls:ident TokenMatch $($indexes:ident)*) => {
        $impls.add_mut(EqlTrait::TokenMatch);
        $crate::to_eql_trait_impls!(@flags $impls $($indexes)*);
    };

    (@flags $impls:ident JsonLike $($indexes:ident)*) => {
        $impls.add_mut(EqlTrait::JsonLike);
        $crate::to_eql_trait_impls!(@flags $impls $($indexes)*);
    };

    (@flags $impls:ident Contain $($indexes:ident)*) => {
        $impls.add_mut(EqlTrait::Contain);
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
    // Explicit v3 domain typname, e.g. `col (EQL("eql_v3_integer_ord_ore"): Ord)`.
    // The token type is parsed from the domain name; use this when a test cares
    // about the token type or the OPE-vs-ORE variant.
    (@add_column $table:ident $column_name:ident (EQL($domain:literal) $(: $trait_:ident $(+ $trait_rest:ident)*)?) ) => {
        {
            let __traits = $crate::to_eql_trait_impls!($($trait_ $($trait_rest)*)?);
            $table.add_column(std::sync::Arc::new($crate::model::Column::eql(
                ::sqltk::parser::ast::Ident::new(stringify!($column_name)),
                __traits,
                $crate::unifier::DomainIdentity::from_domain_name($domain)
                    .expect("EQL(<domain>) must be a valid v3 domain typname"),
            )));
        }
    };
    // Default: no explicit domain — synthesise a canonical `text`-token identity
    // for the given capabilities (fine for the many tests that don't exercise the
    // token type).
    (@add_column $table:ident $column_name:ident (EQL $(: $trait_:ident $(+ $trait_rest:ident)*)?) ) => {
        {
            let __traits = $crate::to_eql_trait_impls!($($trait_ $($trait_rest)*)?);
            $table.add_column(std::sync::Arc::new($crate::model::Column::eql(
                ::sqltk::parser::ast::Ident::new(stringify!($column_name)),
                __traits,
                $crate::unifier::DomainIdentity::canonical($crate::unifier::TokenType::Text, __traits),
            )));
        }
    };
    // A column whose encrypted type this build cannot map, e.g. one left behind
    // by a partial EQL v2 -> v3 migration: `col (UNMAPPABLE("eql_v2_encrypted"))`.
    (@add_column $table:ident $column_name:ident (UNMAPPABLE($column_type:literal)) ) => {
        $table.add_column(std::sync::Arc::new($crate::model::Column::unmappable_encrypted(
            ::sqltk::parser::ast::Ident::new(stringify!($column_name)),
            $column_type,
        )));
    };
    (@add_column $table:ident $column_name:ident (PK) ) => {
        $table.add_column(
            std::sync::Arc::new(
                $crate::model::Column::native(
                    ::sqltk::parser::ast::Ident::new(stringify!($column_name))
                )
            ),
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
