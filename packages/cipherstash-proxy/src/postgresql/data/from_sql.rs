use crate::{
    error::{Error, MappingError},
    postgresql::{format_code::FormatCode, messages::bind::BindParam},
};
use bigdecimal::BigDecimal;
use bytes::BytesMut;
use chrono::NaiveDate;
use cipherstash_client::encryption::Plaintext;
use cipherstash_config::ColumnType;
use postgres_types::FromSql;
use postgres_types::Type;
use rust_decimal::Decimal;
use sqlparser::ast::Value;
use std::str::FromStr;

pub fn bind_param_from_sql(
    param: &BindParam,
    postgres_type: &Type,
    col_type: ColumnType,
) -> Result<Option<Plaintext>, Error> {
    if param.is_null() {
        return Ok(None);
    }

    let pt = match param.format_code {
        FormatCode::Text => text_from_sql(&param.to_string(), postgres_type, col_type),
        FormatCode::Binary => binary_from_sql(&param.bytes, postgres_type, col_type),
    }?;

    Ok(Some(pt))
}

/// Converts a SQL literal to a Plaintext value based on the column type.
/// The [Value] enum represents all the various quoted forms of literals in SQL.
/// This function extracts the inner type and converts it to a Plaintext value.
pub fn literal_from_sql(literal: &Value, col_type: ColumnType) -> Result<Plaintext, MappingError> {
    match literal {
        // All string literal variants
        Value::SingleQuotedString(s)
        | Value::DoubleQuotedString(s)
        | Value::TripleSingleQuotedString(s)
        | Value::TripleDoubleQuotedString(s)
        | Value::EscapedStringLiteral(s)
        | Value::UnicodeStringLiteral(s)
        | Value::TripleSingleQuotedByteStringLiteral(s)
        | Value::TripleDoubleQuotedByteStringLiteral(s)
        | Value::SingleQuotedRawStringLiteral(s)
        | Value::DoubleQuotedRawStringLiteral(s)
        | Value::TripleSingleQuotedRawStringLiteral(s)
        | Value::TripleDoubleQuotedRawStringLiteral(s)
        | Value::NationalStringLiteral(s) => text_from_sql(s, &Type::TEXT, col_type),

        // Dollar quoted strings are a special case of string literals
        Value::DollarQuotedString(s) => text_from_sql(&s.value, &Type::TEXT, col_type),

        // If a boolean was parsed directly map it to a Plaintext::Boolean
        Value::Boolean(b) => Ok(Plaintext::new(*b)),

        // Null values should be mapped to a null Plaintext for the configured column type
        Value::Null => Ok(Plaintext::null_for_column_type(col_type)),

        // Plaintext doesn't have a binary type, so we'll just pass through as a string
        Value::HexStringLiteral(s)
        | Value::SingleQuotedByteStringLiteral(s)
        | Value::DoubleQuotedByteStringLiteral(s) => Ok(Plaintext::new(s.to_owned())),

        // Parsed number types should be mapped according to the postgres_type/column type
        // #[cfg(not(feature = "bigdecimal"))]
        // Value::Number(s, _) => todo!("Number parsed type not implemented"),
        // #[cfg(feature = "bigdecimal")]
        Value::Number(d, _) => decimal_from_sql(d, col_type),

        // TODO: Not sure what the behaviour should be for these
        Value::Placeholder(_) => todo!("Placeholder parsed type not implemented"),
    }
}

/// Converts a string value to a Plaintext value based on input postgres type and target column type.
/// Usually, the input type is a string and the target type is parsed appropriately (for example, a string to a number).
/// However, other input postgres types are possible.
///
/// An example is a timestamp target column ([ColumnType::Timestamp]) where the input type is [Type::DATE].
/// In such cases, this function is called when a [BindParam] is processed with a [FormatCode::Text].
///
/// The following also work!
///
/// ```sql
/// create table example1 (x int, y bigint, z text);
/// insert into example1 VALUES ('100', 10::int, 1000);
///
/// create table example2 (d date);
/// insert into example2 VALUES ('2025-01-01');
/// insert into example2 VALUES ('2025-01-01 15:00:00'::timestamp);
/// ```
///
/// ## Examples
///
/// | Input Type | Target Column Type | Result |
/// |------------|--------------------|--------|
/// | `Type::INT4` | `ColumnType::Utf8Str` | `Plaintext::Utf8Str` |
/// | `Type::INT2` | `ColumnType::Int` | `Plaintext::Int` |
/// | `Type::INT8` | `ColumnType::Int` | `Error`` |
fn text_from_sql(
    val: &str,
    pg_type: &Type,
    col_type: ColumnType,
) -> Result<Plaintext, MappingError> {
    match (pg_type, col_type) {
        // String is is String
        (&Type::TEXT, ColumnType::Utf8Str) => Ok(Plaintext::new(val)),
        // Primitive numeric types are parsed from the string or from types that will fit
        (&Type::TEXT, ColumnType::Float) => parse_str_as_numeric_plaintext::<f64>(val),
        (&Type::TEXT | &Type::INT2, ColumnType::SmallInt) => {
            parse_str_as_numeric_plaintext::<i16>(val)
        }
        (&Type::TEXT | &Type::INT2 | &Type::INT4, ColumnType::Int) => {
            parse_str_as_numeric_plaintext::<i32>(val)
        }
        (&Type::TEXT | &Type::INT2 | &Type::INT4 | &Type::INT8, ColumnType::BigInt) => {
            parse_str_as_numeric_plaintext::<i64>(val)
        }
        (&Type::TEXT | &Type::INT2 | &Type::INT4 | &Type::INT8, ColumnType::BigUInt) => {
            parse_str_as_numeric_plaintext::<u64>(val)
        }
        (
            &Type::TEXT | &Type::BOOL | &Type::INT2 | &Type::INT4 | &Type::INT8,
            ColumnType::Boolean,
        ) => {
            let val = match val {
                "TRUE" | "true" | "t" | "y" | "yes" | "on" | "1" => true,
                "FALSE" | "f" | "false" | "n" | "no" | "off" | "0" => false,
                _ => Err(MappingError::CouldNotParseParameter)?,
            };
            Ok(Plaintext::new(val))
        }
        // NaiveDate::parse_from_str ignores time and offset so these are all valid
        (&Type::TEXT | &Type::DATE | &Type::TIMESTAMP | &Type::TIMESTAMPTZ, ColumnType::Date) => {
            NaiveDate::parse_from_str(val, "%Y-%m-%d")
                .map_err(|_| MappingError::CouldNotParseParameter)
                .map(Plaintext::new)
        }
        (&Type::TEXT | &Type::NUMERIC, ColumnType::Decimal) => Decimal::from_str(val)
            .map_err(|_| MappingError::CouldNotParseParameter)
            .map(Plaintext::new),

        (&Type::TIMESTAMPTZ, _) => {
            unimplemented!("TIMESTAMPTZ")
        }
        (&Type::TEXT, ColumnType::JsonB) => serde_json::from_str::<serde_json::Value>(val)
            .map_err(|_| MappingError::CouldNotParseParameter)
            .map(Plaintext::new),
        (ty, _) => Err(MappingError::UnsupportedParameterType {
            name: ty.name().to_owned(),
            oid: ty.oid(),
        }),
    }
}

/// Converts a binary value to a Plaintext value based on input postgres type and target column type.
/// It is common for clients to send params whose types don't match the column type.
/// For example, an i16 for an INT4/i32 or INT8/i64 value or a string for a numeric value.
fn binary_from_sql(
    bytes: &BytesMut,
    pg_type: &Type,
    col_type: ColumnType,
) -> Result<Plaintext, MappingError> {
    match (pg_type, col_type) {
        (&Type::BOOL, ColumnType::Boolean) => {
            parse_bytes_from_sql::<bool>(bytes, pg_type).map(Plaintext::new)
        }
        (&Type::DATE, ColumnType::Date) => {
            parse_bytes_from_sql::<NaiveDate>(bytes, pg_type).map(Plaintext::new)
        }
        (&Type::FLOAT8, ColumnType::Float) => {
            parse_bytes_from_sql::<f64>(bytes, pg_type).map(Plaintext::new)
        }
        (&Type::INT2, ColumnType::SmallInt) => {
            parse_bytes_from_sql::<i16>(bytes, pg_type).map(Plaintext::new)
        }
        (&Type::TEXT, ColumnType::Utf8Str) => {
            parse_bytes_from_sql::<String>(bytes, pg_type).map(Plaintext::new)
        }

        // INT4 and INT2 can be converted to Int plaintext
        (&Type::INT4, ColumnType::Int) => {
            parse_bytes_from_sql::<i32>(bytes, pg_type).map(Plaintext::new)
        }
        (&Type::INT2, ColumnType::Int) => {
            parse_bytes_from_sql::<i16>(bytes, pg_type).map(|i| Plaintext::new(i as i32))
        }

        // INT8, INT4 and INT2 can be converted to BigInt plaintext
        (&Type::INT8, ColumnType::BigInt) => {
            parse_bytes_from_sql::<i64>(bytes, pg_type).map(Plaintext::new)
        }
        (&Type::INT4, ColumnType::BigInt) => {
            parse_bytes_from_sql::<i32>(bytes, pg_type).map(|i| Plaintext::new(i as i64))
        }
        (&Type::INT2, ColumnType::BigInt) => {
            parse_bytes_from_sql::<i16>(bytes, pg_type).map(|i| Plaintext::new(i as i64))
        }

        // INT8, INT4 and INT2 can be converted to BigUInt plaintext (note the sign change)
        (&Type::INT8, ColumnType::BigUInt) => {
            parse_bytes_from_sql::<i64>(bytes, pg_type).map(|b| Plaintext::new(b as u64))
        }
        (&Type::INT4, ColumnType::BigUInt) => {
            parse_bytes_from_sql::<i32>(bytes, pg_type).map(|b| Plaintext::new(b as u64))
        }
        (&Type::INT2, ColumnType::BigUInt) => {
            parse_bytes_from_sql::<i16>(bytes, pg_type).map(|b| Plaintext::new(b as u64))
        }

        // Even though basically any number can be a decimal, `rust_decimal` only supports converting from NUMERIC
        // Text values will be handled by the text_from_sql function (see below)
        (&Type::NUMERIC, ColumnType::Decimal) => {
            parse_bytes_from_sql::<Decimal>(bytes, pg_type).map(Plaintext::new)
        }

        // If input type is a string but the target column isn't then parse as string and convert
        (&Type::TEXT, _) => parse_bytes_from_sql::<String>(bytes, pg_type)
            .and_then(|val| text_from_sql(&val, pg_type, col_type)),

        (_, ColumnType::JsonB) => {
            parse_bytes_from_sql::<serde_json::Value>(bytes, pg_type).map(Plaintext::new)
        }

        // TODO: timestamps
        (_, ColumnType::Timestamp) => unimplemented!("TIMESTAMPTZ"),

        // Unsupported
        (ty, _) => Err(MappingError::UnsupportedParameterType {
            name: ty.name().to_owned(),
            oid: ty.oid(),
        }),
    }
}

fn parse_bytes_from_sql<T>(bytes: &BytesMut, pg_type: &Type) -> Result<T, MappingError>
where
    T: for<'a> FromSql<'a>,
{
    T::from_sql(pg_type, bytes).map_err(|_| MappingError::CouldNotParseParameter)
}

fn parse_str_as_numeric_plaintext<T>(val: &str) -> Result<Plaintext, MappingError>
where
    T: FromStr + Into<Plaintext>,
{
    val.parse::<T>()
        .map_err(|_| MappingError::CouldNotParseParameter)
        .map(Plaintext::new)
}

fn decimal_from_sql(
    decimal: &BigDecimal,
    column_type: ColumnType,
) -> Result<Plaintext, MappingError> {
    use bigdecimal::ToPrimitive;

    match column_type {
        ColumnType::SmallInt => decimal
            .to_i16()
            .ok_or(MappingError::CouldNotParseParameter)
            .map(Plaintext::new),

        ColumnType::Int => decimal
            .to_i32()
            .ok_or(MappingError::CouldNotParseParameter)
            .map(Plaintext::new),

        ColumnType::BigInt => decimal
            .to_i64()
            .ok_or(MappingError::CouldNotParseParameter)
            .map(Plaintext::new),

        ColumnType::BigUInt => decimal
            .to_u64()
            .ok_or(MappingError::CouldNotParseParameter)
            .map(Plaintext::new),

        ColumnType::Decimal => decimal
            .to_f64()
            .ok_or(MappingError::CouldNotParseParameter)
            .map(Plaintext::new),

        ColumnType::Float => decimal
            .to_f64()
            .ok_or(MappingError::CouldNotParseParameter)
            .map(Plaintext::new),

        ColumnType::Timestamp => decimal
            .to_i64()
            .ok_or(MappingError::CouldNotParseParameter)
            .map(Plaintext::new),

        ColumnType::Utf8Str => Ok(Plaintext::new(decimal.to_string())),

        ColumnType::JsonB => {
            let val: serde_json::Value = serde_json::from_str(&decimal.to_string())
                .map_err(|_| MappingError::CouldNotParseParameter)?;
            Ok(Plaintext::new(val))
        }
        // False 0, True = any other value
        ColumnType::Boolean => {
            let x = decimal
                .to_i8()
                .ok_or(MappingError::CouldNotParseParameter)?;
            let val = x != 0;
            Ok(Plaintext::new(val))
        }
        ColumnType::Date => decimal
            .to_i64()
            .ok_or(MappingError::CouldNotParseParameter)
            .map(Plaintext::new),
    }
}

#[cfg(test)]
mod tests {

    use crate::{
        config::LogConfig,
        log,
        postgresql::{
            data::bind_param_from_sql, format_code::FormatCode, messages::bind::BindParam, Column,
        },
        Identifier,
    };
    use bytes::{BufMut, BytesMut};
    use chrono::NaiveDate;
    use cipherstash_client::encryption::Plaintext;
    use cipherstash_config::{ColumnConfig, ColumnMode, ColumnType};
    use postgres_types::{ToSql, Type};

    fn to_message(s: &[u8]) -> BytesMut {
        BytesMut::from(s)
    }

    fn column(ty: Type) -> Column {
        Column {
            identifier: Identifier::new("table", "column"),
            config: ColumnConfig {
                name: "column".to_owned(),
                in_place: false,
                cast_type: ColumnType::Utf8Str,
                indexes: vec![],
                mode: ColumnMode::PlaintextDuplicate,
            },
            postgres_type: ty,
        }
    }

    #[test]
    pub fn bind_param_to_plaintext_i64() {
        log::init(LogConfig::default());

        // Binary
        let val: i64 = 42;
        let mut bytes = BytesMut::with_capacity(8);
        bytes.put_i64(val);
        let param = BindParam::new(FormatCode::Binary, bytes);

        let pt = bind_param_from_sql(&param, &Type::INT8, ColumnType::BigInt)
            .unwrap()
            .unwrap();
        assert_eq!(pt, Plaintext::BigInt(Some(val)));

        // Text
        let val: i64 = 42;

        let binding = val.to_string();
        let bytes = binding.as_bytes();
        let bytes = BytesMut::from(bytes);

        let param = BindParam::new(FormatCode::Text, bytes);

        let pt = bind_param_from_sql(&param, &Type::INT8, ColumnType::BigInt)
            .unwrap()
            .unwrap();
        assert_eq!(pt, Plaintext::BigInt(Some(val)));
    }

    #[test]
    pub fn bind_param_to_plaintext_boolean() {
        log::init(LogConfig::default());

        // Binary
        let val = true;
        let mut bytes = BytesMut::with_capacity(1);
        bytes.put_u8(true as u8);
        let param = BindParam::new(FormatCode::Binary, bytes);

        let pt = bind_param_from_sql(&param, &Type::BOOL, ColumnType::Boolean)
            .unwrap()
            .unwrap();
        assert_eq!(pt, Plaintext::Boolean(Some(val)));

        // Text
        let val = true;

        let binding = val.to_string();
        let bytes = binding.as_bytes();
        let bytes = BytesMut::from(bytes);

        let param = BindParam::new(FormatCode::Text, bytes);

        let pt = bind_param_from_sql(&param, &Type::BOOL, ColumnType::Boolean)
            .unwrap()
            .unwrap();
        assert_eq!(pt, Plaintext::Boolean(Some(val)));
    }

    #[test]
    pub fn bind_param_to_plaintext_date() {
        log::init(LogConfig::default());

        // // Binary
        let val = NaiveDate::parse_from_str("2025-01-01", "%Y-%m-%d").unwrap();

        let mut bytes = BytesMut::new();
        let _ = val.to_sql_checked(&Type::DATE, &mut bytes);

        let param = BindParam::new(FormatCode::Binary, bytes);

        let pt = bind_param_from_sql(&param, &Type::DATE, ColumnType::Date)
            .unwrap()
            .unwrap();
        assert_eq!(pt, Plaintext::NaiveDate(Some(val)));

        // Text
        let bytes = "2025-01-01".as_bytes();
        let bytes = BytesMut::from(bytes);

        let param = BindParam::new(FormatCode::Text, bytes);

        let pt = bind_param_from_sql(&param, &Type::DATE, ColumnType::Date)
            .unwrap()
            .unwrap();
        assert_eq!(pt, Plaintext::NaiveDate(Some(val)));
    }
}
