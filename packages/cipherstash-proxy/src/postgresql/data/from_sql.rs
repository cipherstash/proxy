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
use tracing::debug;

pub fn bind_param_from_sql(
    param: &BindParam,
    postgres_type: &Type,
    eql_term: EqlTermVariant,
    col_type: ColumnType,
) -> Result<Option<Plaintext>, Error> {
    debug!(target: ENCODING, ?param, ?postgres_type, ?eql_term, ?col_type);

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
        Value::Number(d, _) => {
            // A JSON ordering operand (`col -> sel > 4`) is a scalar SteVec
            // ordering term, encoded as a float like the stored JSON number leaf
            // — NOT a JSON document (`decimal_from_sql` would make a `Json`
            // plaintext, which `SteVecTerm` rejects).
            if eql_term == EqlTermVariant::JsonOrd {
                use bigdecimal::ToPrimitive;
                Some(Plaintext::new(
                    d.to_f64().ok_or(MappingError::CouldNotParseParameter)?,
                ))
            } else {
                Some(decimal_from_sql(d, col_type)?)
            }
        }

        Value::Placeholder(_) => {
            return Err(MappingError::Internal(String::from(
                "placeholder is not a literal",
            )))
        }
    };

    Ok(pt)
}

/// Normalises a JSON field selector to an eJSONPath rooted at `$`.
///
/// `->`/`->>` take a bare field name (`name`), `jsonb_path_query*` takes a path
/// (`nested.title`, or already-rooted `$.nested.title`). The client's
/// `Selector::parse` only accepts the rooted form.
///
/// A path is already rooted if it *starts with* `$`, not merely if it starts
/// with `$.`: `jsonb_path_query` also accepts `$`, `$[0]`, `$["a"]` and
/// `$[*].b`. Re-rooting those would produce `$.$[0]` and friends — a selector
/// that matches nothing rather than erroring.
pub fn json_selector_path(val: &str) -> String {
    compose_json_selector_path(std::slice::from_ref(&val))
}

/// Composes the steps of an accessor chain into one eJSONPath rooted at `$`.
///
/// `col -> 'a' -> 'b'` is the single path `$.a.b` of the root document, not two
/// hops: the intermediate value is an encrypted payload the database cannot
/// traverse, so the whole chain has to be keyed into one selector.
///
/// Every step is normalised the way [`json_selector_path`] normalises a lone
/// one, so the spellings mix freely: `jsonb_path_query_first(col, '$.a') -> 'b'`
/// composes to `$.a.b`, and a subscript step keeps its bracket
/// (`$.a[0]`) rather than gaining a spurious dot.
pub fn compose_json_selector_path(segments: &[&str]) -> String {
    let mut path = String::from("$");

    for segment in segments {
        // A step written as a path of its own is already rooted, and its root
        // is this path so far — drop the `$` and splice the remainder on.
        let (rooted, rest) = match segment.strip_prefix('$') {
            Some(rest) => (true, rest),
            None => (false, *segment),
        };

        // A bare `$` selects the document itself and adds no step.
        if rooted && rest.is_empty() {
            continue;
        }

        if !rest.starts_with('.') && !rest.starts_with('[') {
            path.push('.');
        }

        path.push_str(rest);
    }

    path
}

/// Builds the composition input for a fused JSON value selector:
/// `{"path": <jsonpath>, "value": <scalar>}`.
///
/// This is the one place the operands of a JSON field equality become one
/// encrypted operand. `QueryOp::SteVecValueSelector` MACs the path and the
/// canonicalised value together into a single selector; its presence in the
/// stored `sv` is the equality match. The client applies the column's term
/// filters (e.g. downcase) to `value` as part of that, so case-insensitive
/// columns work unchanged here.
///
/// `path` arrives as the steps of the accessor chain, already resolved to text;
/// they are composed into one eJSONPath here so that a chained accessor keys the
/// same needle as the equivalent single-step path.
///
/// `value` must be a scalar. A single value selector is only injective for
/// scalars — a container MACs just its structural tag, so every object at a path
/// would collapse to one selector. The client rejects those; rejecting here too
/// gives a message naming the query shape rather than the encryption internals.
pub fn json_value_selector_plaintext(
    path: &[&str],
    value: serde_json::Value,
) -> Result<Plaintext, MappingError> {
    if value.is_object() || value.is_array() {
        debug!(
            target: ENCODING,
            msg = "Encrypted JSON equality requires a scalar value",
            ?path,
            ?value
        );
        return Err(MappingError::CouldNotParseParameter);
    }

    Ok(Plaintext::new(serde_json::json!({
        "path": compose_json_selector_path(path),
        "value": value,
    })))
}

/// The JSON value a literal carries, for fusing into a value selector.
///
/// The value half of `col -> sel = value` is written as a quoted JSON scalar
/// (`= '"B"'`, `= '3'`) or as a bare SQL number (`= 3`). A quoted string that is
/// not valid JSON is taken as the string itself, so `= 'B'` behaves like
/// `= '"B"'`.
pub fn literal_json_value(literal: &Value) -> Result<Option<serde_json::Value>, MappingError> {
    let value = match literal {
        Value::Null => None,

        Value::Number(d, _) => Some(
            serde_json::from_str::<serde_json::Value>(&d.to_string())
                .map_err(|_| MappingError::CouldNotParseParameter)?,
        ),

        Value::Boolean(b) => Some(serde_json::Value::Bool(*b)),

        Value::SingleQuotedString(s)
        | Value::DoubleQuotedString(s)
        | Value::TripleSingleQuotedString(s)
        | Value::TripleDoubleQuotedString(s)
        | Value::EscapedStringLiteral(s)
        | Value::UnicodeStringLiteral(s)
        | Value::NationalStringLiteral(s) => Some(
            serde_json::from_str::<serde_json::Value>(s)
                .unwrap_or_else(|_| serde_json::Value::String(s.to_owned())),
        ),

        Value::DollarQuotedString(s) => Some(
            serde_json::from_str::<serde_json::Value>(&s.value)
                .unwrap_or_else(|_| serde_json::Value::String(s.value.to_owned())),
        ),

        _ => return Err(MappingError::CouldNotParseParameter),
    };

    Ok(value)
}

/// The JSON value a bind param carries, for fusing into a value selector.
///
/// Mirrors [`literal_json_value`] for the extended protocol: a jsonb param
/// arrives either as its text rendering (`4`, `"C"`) or as binary jsonb. A text
/// payload that is not valid JSON is taken as the string itself.
pub fn bind_param_json_value(
    param: &BindParam,
    postgres_type: &Type,
) -> Result<Option<serde_json::Value>, MappingError> {
    if param.is_null() {
        return Ok(None);
    }

    let value = match param.format_code {
        FormatCode::Text => {
            let text = param.to_string();
            serde_json::from_str::<serde_json::Value>(&text)
                .unwrap_or(serde_json::Value::String(text))
        }
        FormatCode::Binary => binary_json_value(&param.bytes, postgres_type)?,
    };

    Ok(Some(value))
}

/// Whether `ty` is one of PostgreSQL's textual types, whose binary
/// representation is the string's own bytes rather than anything JSON-shaped.
fn is_textual(ty: &Type) -> bool {
    matches!(
        *ty,
        Type::TEXT | Type::VARCHAR | Type::BPCHAR | Type::NAME | Type::UNKNOWN
    )
}

/// The JSON value a binary-format operand carries.
///
/// `serde_json::Value` only decodes JSON and JSONB, so a textual type has to be
/// read as a string first — otherwise ordinary bytes such as `Alice` are handed
/// to `serde_json::Value::from_sql` and rejected outright, even though the same
/// operand in text format would have been accepted.
///
/// Once read, it gets exactly the treatment the text format gets: parsed as
/// JSON, and taken as the string itself when that fails, so `Alice` behaves like
/// `"Alice"`.
fn binary_json_value(
    bytes: &BytesMut,
    postgres_type: &Type,
) -> Result<serde_json::Value, MappingError> {
    if is_textual(postgres_type) {
        let text = parse_bytes_from_sql::<String>(bytes, postgres_type)
            .map_err(|_| MappingError::CouldNotParseParameter)?;

        return Ok(serde_json::from_str::<serde_json::Value>(&text)
            .unwrap_or(serde_json::Value::String(text)));
    }

    parse_bytes_from_sql::<serde_json::Value>(bytes, postgres_type)
        .map_err(|_| MappingError::CouldNotParseParameter)
}

/// A JSON ordering operand (`EqlTerm::JsonOrd`) arriving as a jsonb param
/// carries a single scalar. Encode a number as a float (matching the stored
/// JSON number leaf's SteVec `op` encoding) and a string as text — the only
/// scalar shapes `SteVecTerm` accepts (a full JSON value is rejected).
fn json_ord_scalar_plaintext(value: serde_json::Value) -> Result<Plaintext, MappingError> {
    match value {
        serde_json::Value::Number(n) => n
            .as_f64()
            .ok_or(MappingError::CouldNotParseParameter)
            .map(Plaintext::new),
        serde_json::Value::String(s) => Ok(Plaintext::new(s)),
        _ => Err(MappingError::CouldNotParseParameter),
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
/// | `Type::INT4` | `ColumnType::Text` | `Plaintext::Text` |
/// | `Type::INT2` | `ColumnType::Int` | `Plaintext::Int` |
/// | `Type::INT8` | `ColumnType::Int` | `Error`` |
fn text_from_sql(
    val: &str,
    eql_term: EqlTermVariant,
    col_type: ColumnType,
) -> Result<Plaintext, MappingError> {
    debug!(target: ENCODING, ?val, ?eql_term, ?col_type);

    match (eql_term, col_type) {
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::Text) => {
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

        // If JSONB, JSONPATH values are treated as strings
        (EqlTermVariant::JsonPath | EqlTermVariant::JsonAccessor, ColumnType::Json) => {
            Ok(Plaintext::new(json_selector_path(val)))
        }
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::Json) => {
            serde_json::from_str::<serde_json::Value>(val)
                .map_err(|_| MappingError::CouldNotParseParameter)
                .map(Plaintext::new)
        }
        // A JSON ordering operand reaches here in two textual shapes: a bare SQL
        // literal (`col -> sel > '4'` / `> 'C'` → `4` / `C`) and a text-format
        // jsonb param (`4` / `"C"`, the value's jsonb rendering). Parse as JSON to
        // recover the scalar type and its content: a number encodes as a float
        // (matching the stored leaf's `for_json_value` SteVec `op`), a JSON string
        // as its unquoted text. A bare word (`C`) is not valid JSON, so fall back
        // to raw text. Mirrors `json_ord_scalar_plaintext` on the binary param path.
        (EqlTermVariant::JsonOrd, ColumnType::Json) => {
            match serde_json::from_str::<serde_json::Value>(val) {
                Ok(value @ (serde_json::Value::Number(_) | serde_json::Value::String(_))) => {
                    json_ord_scalar_plaintext(value)
                }
                _ => Ok(Plaintext::new(val)),
            }
        }
        (EqlTermVariant::Tokenized, ColumnType::Text) => Ok(Plaintext::new(val)),

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
        (EqlTermVariant::Full | EqlTermVariant::Partial, ColumnType::Text, _) => {
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
        (EqlTermVariant::JsonPath, ColumnType::Json, &Type::JSONPATH) => {
            parse_bytes_from_sql::<String>(bytes, pg_type)
                .map(|val| Plaintext::new(json_selector_path(&val)))
        }
        (EqlTermVariant::JsonAccessor, ColumnType::Json, &Type::TEXT | &Type::VARCHAR) => {
            parse_bytes_from_sql::<String>(bytes, pg_type)
                .map(|val| Plaintext::new(json_selector_path(&val)))
        }
        // A JSON ordering operand (`col -> sel > $2`) arrives as a jsonb scalar;
        // encode it as the scalar shape SteVecTerm accepts (number → float,
        // string → text).
        (EqlTermVariant::JsonOrd, ColumnType::Json, _) => {
            // Via `binary_json_value` rather than straight to `serde_json`: this
            // arm matches any incoming type, so a textual operand has to be read
            // as a string before it can be parsed as JSON.
            binary_json_value(bytes, pg_type).and_then(json_ord_scalar_plaintext)
        }
        // Python psycopg sends JSON/B as BYTEA
        (
            EqlTermVariant::Full | EqlTermVariant::Partial,
            ColumnType::Json,
            &Type::JSON | &Type::JSONB | &Type::BYTEA,
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

        ColumnType::Text => Ok(Plaintext::new(decimal.to_string())),

        ColumnType::Json => {
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
mod binary_json_value_tests {
    use super::*;
    use crate::postgresql::{format_code::FormatCode, messages::bind::BindParam};
    use bytes::BytesMut;

    fn binary_param(bytes: &[u8]) -> BindParam {
        BindParam::new(FormatCode::Binary, BytesMut::from(bytes))
    }

    /// A textual operand in binary format is the string's own bytes. Reading it
    /// as JSON rejects anything that is not JSON-shaped, so it is read as a
    /// string first and then given the text format's treatment.
    #[test]
    fn binary_textual_operand_is_read_as_a_string() {
        for ty in [Type::TEXT, Type::VARCHAR, Type::BPCHAR, Type::NAME] {
            let param = binary_param(b"Alice");

            assert_eq!(
                Some(serde_json::Value::String("Alice".to_string())),
                bind_param_json_value(&param, &ty).unwrap(),
                "unexpected decoding of a binary {ty} operand"
            );
        }
    }

    /// A textual operand that *is* valid JSON still parses as JSON, so a client
    /// sending `"Alice"` or `42` as text gets the same value as one sending
    /// jsonb.
    #[test]
    fn binary_textual_operand_that_is_json_parses_as_json() {
        assert_eq!(
            Some(serde_json::Value::String("Alice".to_string())),
            bind_param_json_value(&binary_param(b"\"Alice\""), &Type::TEXT).unwrap()
        );

        assert_eq!(
            Some(serde_json::json!(42)),
            bind_param_json_value(&binary_param(b"42"), &Type::TEXT).unwrap()
        );
    }

    /// The jsonb path is untouched: version header byte, then the JSON text.
    #[test]
    fn binary_jsonb_operand_still_decodes_as_jsonb() {
        let mut bytes = BytesMut::from(&b"\x01"[..]);
        bytes.extend_from_slice(b"{\"a\":1}");

        assert_eq!(
            Some(serde_json::json!({"a": 1})),
            bind_param_json_value(&BindParam::new(FormatCode::Binary, bytes), &Type::JSONB)
                .unwrap()
        );
    }

    /// A NULL param carries no value at all.
    #[test]
    fn null_param_has_no_value() {
        assert_eq!(
            None,
            bind_param_json_value(&BindParam::null(), &Type::TEXT).unwrap()
        );
    }

    /// A chain of steps composes into ONE path, so `col -> 'a' -> 'b'` keys the
    /// same needle as the equivalent `jsonb_path_query_first(col, '$.a.b')`.
    #[test]
    fn accessor_chain_composes_into_one_path() {
        assert_eq!("$.a.b", compose_json_selector_path(&["a", "b"]));
        assert_eq!(
            "$.a.b.c.d",
            compose_json_selector_path(&["a", "b", "c", "d"])
        );
        assert_eq!(
            compose_json_selector_path(&["$.a.b"]),
            compose_json_selector_path(&["a", "b"]),
            "the spellings of one path must agree"
        );
    }

    /// A step already written as a path is rooted at the path so far, not at a
    /// second `$`.
    #[test]
    fn a_rooted_step_splices_onto_the_path_so_far() {
        assert_eq!("$.a.b", compose_json_selector_path(&["$.a", "b"]));
        assert_eq!("$.a.b", compose_json_selector_path(&["a", "$.b"]));
        assert_eq!("$.a[0].b", compose_json_selector_path(&["a", "$[0]", "b"]));
        // A bare `$` is the document itself and adds no step.
        assert_eq!("$.a", compose_json_selector_path(&["$", "a"]));
    }

    /// A single step composes exactly as it always did.
    #[test]
    fn a_single_step_keeps_its_rooting_rules() {
        for path in [
            "name",
            "nested.title",
            "$.nested.title",
            "$",
            "$[0]",
            "$[*].b",
        ] {
            assert_eq!(
                json_selector_path(path),
                compose_json_selector_path(&[path]),
                "unexpected composition of `{path}`"
            );
        }

        assert_eq!("$.name", json_selector_path("name"));
        assert_eq!("$[0]", json_selector_path("$[0]"));
    }

    /// The needle is keyed on the composed path, and containers are rejected
    /// whatever the path's shape.
    #[test]
    fn a_needle_is_keyed_on_the_composed_path() {
        assert_eq!(
            Plaintext::new(serde_json::json!({"path": "$.a.b", "value": "v"})),
            json_value_selector_plaintext(&["a", "b"], serde_json::json!("v")).unwrap()
        );

        assert!(json_value_selector_plaintext(&["a", "b"], serde_json::json!({"x": 1})).is_err());
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
                cast_type: ColumnType::Text,
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
