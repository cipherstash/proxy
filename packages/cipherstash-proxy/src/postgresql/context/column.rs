use cipherstash_client::schema::{ColumnConfig, ColumnType};
use eql_mapper::EqlTermVariant;
use postgres_types::Type;

use crate::Identifier;

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub identifier: Identifier,
    pub config: ColumnConfig,
    pub postgres_type: Type,
    pub eql_term: EqlTermVariant,
}

impl Column {
    pub fn new(
        identifier: Identifier,
        config: ColumnConfig,
        postgres_type: Option<Type>,
        eql_term: EqlTermVariant,
    ) -> Column {
        let postgres_type =
            postgres_type.unwrap_or(column_type_to_postgres_type(&config.cast_type, eql_term));

        Column {
            identifier,
            config,
            postgres_type,
            eql_term,
        }
    }

    pub fn table_name(&self) -> String {
        self.identifier.table.to_owned()
    }

    pub fn column_name(&self) -> String {
        self.identifier.column.to_owned()
    }

    pub fn oid(&self) -> u32 {
        self.postgres_type.oid()
    }

    pub fn cast_type(&self) -> ColumnType {
        self.config.cast_type
    }

    pub fn eql_term(&self) -> EqlTermVariant {
        self.eql_term
    }

    pub fn is_encryptable(&self) -> bool {
        matches!(
            self.eql_term,
            EqlTermVariant::Full | EqlTermVariant::Partial
        )
    }
}

///
/// Maps a configured index type to a Postgres Type
///
/// JSONAccessors are mapped to a string for the client, but are JSONB for the server
///
fn column_type_to_postgres_type(
    col_type: &ColumnType,
    eql_term: EqlTermVariant,
) -> postgres_types::Type {
    match (col_type, eql_term) {
        (ColumnType::Boolean, _) => postgres_types::Type::BOOL,
        (ColumnType::BigInt, _) => postgres_types::Type::INT8,
        (ColumnType::BigUInt, _) => postgres_types::Type::INT8,
        (ColumnType::Date, _) => postgres_types::Type::DATE,
        (ColumnType::Decimal, _) => postgres_types::Type::NUMERIC,
        (ColumnType::Float, _) => postgres_types::Type::FLOAT8,
        (ColumnType::Int, _) => postgres_types::Type::INT4,
        (ColumnType::SmallInt, _) => postgres_types::Type::INT2,
        (ColumnType::Timestamp, _) => postgres_types::Type::TIMESTAMPTZ,
        (ColumnType::Utf8Str, _) => postgres_types::Type::TEXT,
        (ColumnType::JsonB, EqlTermVariant::JsonAccessor) => postgres_types::Type::TEXT,
        (ColumnType::JsonB, _) => postgres_types::Type::JSONB,
    }
}
