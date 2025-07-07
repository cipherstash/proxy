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
            postgres_type.unwrap_or(column_type_to_postgres_type(&config.cast_type));

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
        // self.postgres_type != postgres_types::Type::JSONPATH
        matches!(
            self.eql_term,
            EqlTermVariant::Full | EqlTermVariant::Partial
        )
    }
}

fn column_type_to_postgres_type(col_type: &ColumnType) -> postgres_types::Type {
    match col_type {
        ColumnType::Boolean => postgres_types::Type::BOOL,
        ColumnType::BigInt => postgres_types::Type::INT8,
        ColumnType::BigUInt => postgres_types::Type::INT8,
        ColumnType::Date => postgres_types::Type::DATE,
        ColumnType::Decimal => postgres_types::Type::NUMERIC,
        ColumnType::Float => postgres_types::Type::FLOAT8,
        ColumnType::Int => postgres_types::Type::INT4,
        ColumnType::SmallInt => postgres_types::Type::INT2,
        ColumnType::Timestamp => postgres_types::Type::TIMESTAMPTZ,
        ColumnType::Utf8Str => postgres_types::Type::TEXT,
        ColumnType::JsonB => postgres_types::Type::JSONB,
    }
}
