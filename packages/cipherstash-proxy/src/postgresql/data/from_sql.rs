use crate::{
    error::{Error, MappingError},
    log::ENCODING,
    postgresql::{format_code::FormatCode, messages::bind::BindParam},
};
use bigdecimal::BigDecimal;
use bytes::BytesMut;
use chrono::NaiveDate;
use cipherstash_client::{encryption::Plaintext, schema::ColumnType};
use eql_mapper::EqlTermVariant;
use postgres_types::FromSql;
use postgres_types::Type;
use rust_decimal::Decimal;
use sqltk::parser::ast::Value;
use std::str::FromStr;
use tracing::{debug, error};

pub fn bind_param_from_sql(
    param: &BindParam,
    postgres_type: &Type,
    eql_term: EqlTermVariant,
    col_type: ColumnType,
) -> Result<Option<Plaintext>, Error> {
    error!(target: ENCODING, ?param, ?postgres_type, ?eql_term, ?col_type);

    if param.is_null() {
        return Ok(None);
    }

    let pt = match param.format_code {
        FormatCode::Text => text_from_sql(&param.to_string(), eql_term, col_type),
        FormatCode::Binary => binary_from_sql(&param.bytes, postgres_type, eql_term, col_type),
    }?;

    Ok(Some(pt))
}

/// Converts a SQL literal to a Plaintext value based on the column type.
/// Returns Some(Plaintext) or None if the literal is NULL.
/// The [Value] enum represents all the various quoted forms of literals in SQL.
/// This function extracts the inner type and converts it to a Plaintext value.
pub fn literal_from_sql(
    literal: &Value,
    eql_term: EqlTermVariant,
    col_type: ColumnType,
) -> Result<Option<Plaintext>, MappingError> {
    debug!(target: ENCODING, ?literal, ?eql_term, ?col_type);
    let pt = match literal {
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
        | Value::NationalStringLiteral(s) => Some(text_from_sql(s, eql_term, col_type)?),

        // Dollar quoted strings are a special case of string literals
        Value::DollarQuotedString(s) => Some(text_from_sql(&s.value, eql_term, col_type)?),

        // If a boolean was parsed directly map it to a Plaintext::Boolean
        Value::Boolean(b) => Some(Plaintext::new(*b)),

        // TODO: encrypted nulls
        // Null values should be mapped to a null Plaintext for the configured column type
        // Value::Null => Ok(Plaintext::null_for_column_type(col_type)),
        Value::Null => None,

        // Plaintext doesn't have a binary type, so we'll just pass through as a string
        Value::HexStringLiteral(s)
        | Value::SingleQuotedByteStringLiteral(s)
        | Value::DoubleQuotedByteStringLiteral(s) => Some(Plaintext::new(s.to_owned())),

        // Parsed number types should be mapped according to the postgres_type/column type
        // #[cfg(not(feature = "bigdecimal"))]
        // Value::Number(s, _) => todo!("Number parsed type not implemented"),
        // #[cfg(feature = "bigdecimal")]
        Value::Number(d, _) => Some(decimal_from_sql(d, col_type)?),

        Value::Placeholder(_) => {
            return Err(MappingError::Internal(String::from(
                "placeholder is not a literal",
            )))
        }
    };

    Ok(pt)
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
    eql_term: EqlTermVariant,
    col_type: ColumnType,
) -> Result<Plaintext, MappingError> {
    debug!(target: ENCODING, ?val, ?eql_term, ?col_type);

    match (eql_term, col_type) {
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::Utf8Str) => {
            Ok(Plaintext::new(val))
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::Float) => {
            parse_str_as_numeric_plaintext::<f64>(val)
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::SmallInt) => {
            parse_str_as_numeric_plaintext::<i16>(val)
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::Int) => {
            parse_str_as_numeric_plaintext::<i32>(val)
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::BigInt) => {
            parse_str_as_numeric_plaintext::<i64>(val)
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::BigUInt) => {
            parse_str_as_numeric_plaintext::<u64>(val)
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::Boolean) => {
            let val = match val {
                "TRUE" | "true" | "t" | "y" | "yes" | "on" | "1" => true,
                "FALSE" | "f" | "false" | "n" | "no" | "off" | "0" => false,
                _ => Err(MappingError::CouldNotParseParameter)?,
            };
            Ok(Plaintext::new(val))
        }
        // NaiveDate::parse_from_str ignores time and offset so these are all valid
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::Date) => {
            NaiveDate::parse_from_str(val, "%Y-%m-%d")
                .map_err(|_| MappingError::CouldNotParseParameter)
                .map(Plaintext::new)
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::Decimal) => {
            Decimal::from_str(val)
                .map_err(|_| MappingError::CouldNotParseParameter)
                .map(Plaintext::new)
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::Timestamp) => {
            unimplemented!("Timestamp")
        }

        // postgres_type: Jsonpath, eql_term: Full, col_type: JsonB

        // If JSONB, JSONPATH values are treated as strings
        (EqlTermVariant::JsonPath | EqlTermVariant::JsonAccessor, ColumnType::JsonB) => {
            Ok(Plaintext::new(val))
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::JsonB) => {
            serde_json::from_str::<serde_json::Value>(val)
                .map_err(|_| MappingError::CouldNotParseParameter)
                .map(Plaintext::new)
        }
        (EqlTermVariant::Tokenized, ColumnType::Utf8Str) => Ok(Plaintext::new(val)),

        (eql_term, col_type) => Err(MappingError::UnsupportedParameterType {
            eql_term,
            column_type: col_type,
        }),
    }
}

/// Converts a binary value to a Plaintext value based on input postgres type and target column type.
/// It is common for clients to send params whose types don't match the column type.
/// For example, an i16 for an INT4/i32 or INT8/i64 value or a string for a numeric value.
fn binary_from_sql(
    bytes: &BytesMut,
    pg_type: &Type,
    eql_term: EqlTermVariant,
    col_type: ColumnType,
) -> Result<Plaintext, MappingError> {
    debug!(target: ENCODING, ?pg_type, ?eql_term, ?col_type);

    match (eql_term, col_type, pg_type) {
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::Utf8Str, _) => {
            parse_bytes_from_sql::<String>(bytes, pg_type).map(Plaintext::new)
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::Boolean, _) => {
            parse_bytes_from_sql::<bool>(bytes, pg_type).map(Plaintext::new)
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::Date, _) => {
            parse_bytes_from_sql::<NaiveDate>(bytes, pg_type).map(Plaintext::new)
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::Float, _) => {
            parse_bytes_from_sql::<f64>(bytes, pg_type).map(Plaintext::new)
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::SmallInt, _) => {
            parse_bytes_from_sql::<i16>(bytes, pg_type).map(Plaintext::new)
        }
        // INT4 and INT2 can be converted to Int plaintext
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::Int, &Type::INT4) => {
            parse_bytes_from_sql::<i32>(bytes, pg_type).map(Plaintext::new)
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::Int, &Type::INT2) => {
            parse_bytes_from_sql::<i16>(bytes, pg_type).map(|i| Plaintext::new(i as i32))
        }

        // INT8, INT4 and INT2 can be converted to BigInt plaintext
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::BigInt, &Type::INT8) => {
            parse_bytes_from_sql::<i64>(bytes, pg_type).map(Plaintext::new)
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::BigInt, &Type::INT4) => {
            parse_bytes_from_sql::<i32>(bytes, pg_type).map(|i| Plaintext::new(i as i64))
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::BigInt, &Type::INT2) => {
            parse_bytes_from_sql::<i16>(bytes, pg_type).map(|i| Plaintext::new(i as i64))
        }

        // INT8, INT4 and INT2 can be converted to BigUInt plaintext (note the sign change)
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::BigUInt, &Type::INT8) => {
            parse_bytes_from_sql::<i64>(bytes, pg_type).map(|b| Plaintext::new(b as u64))
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::BigUInt, &Type::INT4) => {
            parse_bytes_from_sql::<i32>(bytes, pg_type).map(|b| Plaintext::new(b as u64))
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::BigUInt, &Type::INT2) => {
            parse_bytes_from_sql::<i16>(bytes, pg_type).map(|b| Plaintext::new(b as u64))
        }

        // Even though basically any number can be a decimal, `rust_decimal` only supports converting from NUMERIC
        // Text values will be handled by the text_from_sql function (see below)
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::Decimal, &Type::NUMERIC) => {
            parse_bytes_from_sql::<Decimal>(bytes, pg_type).map(Plaintext::new)
        }

        // If JSONB, JSONPATH values are treated as strings
        (EqlTermVariant::JsonPath, ColumnType::JsonB, &Type::JSONPATH) => {
            parse_bytes_from_sql::<String>(bytes, pg_type).map(Plaintext::new)
        }
        (EqlTermVariant::JsonAccessor, ColumnType::JsonB, &Type::TEXT | &Type::VARCHAR) => {
            parse_bytes_from_sql::<String>(bytes, pg_type).map(Plaintext::new)
        }
        (
            EqlTermVariant::Full | EqlTermVariant::Partial,
            ColumnType::JsonB,
            &Type::JSON | &Type::JSONB,
        ) => parse_bytes_from_sql::<serde_json::Value>(bytes, pg_type).map(Plaintext::new),

        // TODO: timestamps
        (_, ColumnType::Timestamp, &Type::TIMESTAMPTZ) => unimplemented!("TIMESTAMPTZ"),

        // If input type is a string but the target column isn't then parse as string and convert
        // (&Type::TEXT, _) => parse_bytes_from_sql::<String>(bytes, pg_type)
        //     .and_then(|val| text_from_sql(&val, pg_type, col_type)),

        // If input type is a string but the target column isn't then parse as string and convert
        (_, _, &Type::TEXT | &Type::VARCHAR) => parse_bytes_from_sql::<String>(bytes, pg_type)
            .and_then(|val| text_from_sql(&val, EqlTermVariant::Full, col_type)),

        (eql_term, col_type, _) => Err(MappingError::UnsupportedParameterType {
            eql_term,
            column_type: col_type,
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
    use cipherstash_client::{
        encryption::Plaintext,
        schema::{ColumnConfig, ColumnMode, ColumnType},
    };
    use eql_mapper::EqlTermVariant;
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
            eql_term: EqlTermVariant::Full,
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

        let pt = bind_param_from_sql(
            &param,
            &Type::INT8,
            EqlTermVariant::Full,
            ColumnType::BigInt,
        )
        .unwrap()
        .unwrap();
        assert_eq!(pt, Plaintext::BigInt(Some(val)));

        // Text
        let val: i64 = 42;

        let binding = val.to_string();
        let bytes = binding.as_bytes();
        let bytes = BytesMut::from(bytes);

        let param = BindParam::new(FormatCode::Text, bytes);

        let pt = bind_param_from_sql(
            &param,
            &Type::INT8,
            EqlTermVariant::Full,
            ColumnType::BigInt,
        )
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

        let pt = bind_param_from_sql(
            &param,
            &Type::BOOL,
            EqlTermVariant::Full,
            ColumnType::Boolean,
        )
        .unwrap()
        .unwrap();
        assert_eq!(pt, Plaintext::Boolean(Some(val)));

        // Text
        let val = true;

        let binding = val.to_string();
        let bytes = binding.as_bytes();
        let bytes = BytesMut::from(bytes);

        let param = BindParam::new(FormatCode::Text, bytes);

        let pt = bind_param_from_sql(
            &param,
            &Type::BOOL,
            EqlTermVariant::Full,
            ColumnType::Boolean,
        )
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

        let pt = bind_param_from_sql(&param, &Type::DATE, EqlTermVariant::Full, ColumnType::Date)
            .unwrap()
            .unwrap();
        assert_eq!(pt, Plaintext::NaiveDate(Some(val)));

        // Text
        let bytes = "2025-01-01".as_bytes();
        let bytes = BytesMut::from(bytes);

        let param = BindParam::new(FormatCode::Text, bytes);

        let pt = bind_param_from_sql(&param, &Type::DATE, EqlTermVariant::Full, ColumnType::Date)
            .unwrap()
            .unwrap();
        assert_eq!(pt, Plaintext::NaiveDate(Some(val)));
    }
}
