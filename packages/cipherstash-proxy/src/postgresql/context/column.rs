use cipherstash_client::schema::{ColumnConfig, ColumnType};
use postgres_types::Type;

use crate::Identifier;

#[derive(Debug, Clone, PartialEq)]
pub struct Column {
    pub identifier: Identifier,
    pub config: ColumnConfig,
    pub postgres_type: Type,
}

impl Column {
    pub fn new(identifier: Identifier, config: ColumnConfig) -> Column {
        let postgres_type = column_type_to_postgres_type(&config.cast_type);
        Column {
            identifier,
            config,
            postgres_type,
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

    pub fn postgres_type_name(&self) -> &str {
        self.postgres_type.name()
    }

    pub fn cast_type(&self) -> ColumnType {
        self.config.cast_type
    }

    pub fn is_param_type(&self, param_type: &Type) -> bool {
        param_type == &self.postgres_type
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
