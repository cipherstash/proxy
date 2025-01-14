use crate::{
    error::{Error, MappingError},
    log::MAPPER,
    postgresql::{format_code::FormatCode, messages::bind::BindParam},
};
use bytes::BytesMut;
use chrono::NaiveDate;
use cipherstash_client::encryption::Plaintext;
use postgres_types::FromSql;
use postgres_types::Type;
use rust_decimal::Decimal;
use std::str::FromStr;
use tracing::{debug, info};

pub fn from_sql(param: &BindParam, postgres_type: &Type) -> Result<Option<Plaintext>, Error> {
    if param.is_null() {
        return Ok(None);
    }

    let pt = match param.format_code {
        FormatCode::Text => text_from_sql(&param.to_string(), postgres_type),
        FormatCode::Binary => binary_from_sql(&param.bytes, postgres_type),
    }?;

    Ok(Some(pt))
}

fn text_from_sql(val: &str, postgres_type: &Type) -> Result<Plaintext, Error> {
    match postgres_type {
        &Type::BOOL => {
            let val = match val {
                "TRUE" | "true" | "t" | "y" | "yes" | "on" | "1" => true,
                "FALSE" | "f" | "false" | "n" | "no" | "off" | "0" => false,
                _ => Err(MappingError::CouldNotParseParameter)?,
            };
            Ok(Plaintext::Boolean(Some(val)))
        }
        &Type::DATE => {
            let val = NaiveDate::parse_from_str(val, "%Y-%m-%d")?;
            Ok(Plaintext::NaiveDate(Some(val)))
        }
        &Type::FLOAT8 => {
            let val = val.parse()?;
            Ok(Plaintext::Float(Some(val)))
        }
        &Type::INT2 => {
            let val = val.parse()?;
            Ok(Plaintext::SmallInt(Some(val)))
        }
        &Type::INT4 => {
            let val = val.parse()?;
            Ok(Plaintext::Int(Some(val)))
        }
        &Type::INT8 => {
            let val = val.parse()?;
            Ok(Plaintext::BigInt(Some(val)))
        }
        &Type::NUMERIC => {
            let val = Decimal::from_str(val)?;
            Ok(Plaintext::Decimal(Some(val)))
        }
        &Type::TEXT => {
            let val = val.to_owned();
            Ok(Plaintext::Utf8Str(Some(val)))
        }
        &Type::TIMESTAMPTZ => {
            unimplemented!("TIMESTAMPTZ")
        }
        &Type::JSONB => {
            unimplemented!("JSONB")
        }
        ty => Err(MappingError::UnsupportedParameterType {
            name: ty.name().to_owned(),
            oid: ty.oid(),
        }
        .into()),
    }
}

fn binary_from_sql(bytes: &BytesMut, postgres_type: &Type) -> Result<Plaintext, Error> {
    match postgres_type {
        &Type::BOOL => {
            let val = <bool>::from_sql(&Type::BOOL, bytes)
                .map_err(|_| MappingError::CouldNotParseParameter)?;
            Ok(Plaintext::Boolean(Some(val)))
        }
        &Type::DATE => {
            let val = <NaiveDate>::from_sql(&Type::DATE, bytes)
                .map_err(|_| MappingError::CouldNotParseParameter)?;
            Ok(Plaintext::NaiveDate(Some(val)))
        }
        &Type::FLOAT8 => {
            let val = <f64>::from_sql(&Type::FLOAT8, bytes)
                .map_err(|_| MappingError::CouldNotParseParameter)?;
            Ok(Plaintext::Float(Some(val)))
        }
        &Type::INT2 => {
            debug!(target = MAPPER, "BINARY INT2");
            debug!(target = MAPPER, "{bytes:?}");
            let val = <i16>::from_sql(&Type::INT2, bytes);

            info!(target = MAPPER, "{val:?}");

            let val = val.map_err(|_| MappingError::CouldNotParseParameter)?;

            Ok(Plaintext::SmallInt(Some(val)))
        }
        &Type::INT4 => {
            let val = <i32>::from_sql(&Type::INT4, bytes)
                .map_err(|_| MappingError::CouldNotParseParameter)?;
            Ok(Plaintext::Int(Some(val)))
        }
        &Type::INT8 => {
            let val = <i64>::from_sql(&Type::INT8, bytes)
                .map_err(|_| MappingError::CouldNotParseParameter)?;
            Ok(Plaintext::BigInt(Some(val)))
        }
        &Type::NUMERIC => {
            let val = <Decimal>::from_sql(&Type::NUMERIC, bytes)
                .map_err(|_| MappingError::CouldNotParseParameter)?;
            Ok(Plaintext::Decimal(Some(val)))
        }
        &Type::TEXT => {
            let val = <String>::from_sql(&Type::TEXT, bytes)
                .map_err(|_| MappingError::CouldNotParseParameter)?;
            Ok(Plaintext::Utf8Str(Some(val)))
        }
        &Type::TIMESTAMPTZ => {
            unimplemented!("TIMESTAMPTZ")
        }
        &Type::JSONB => {
            unimplemented!("JSONB")
        }
        ty => Err(MappingError::UnsupportedParameterType {
            name: ty.name().to_owned(),
            oid: ty.oid(),
        }
        .into()),
    }
}

#[cfg(test)]
mod tests {

    use crate::{
        log,
        postgresql::{data::from_sql, format_code::FormatCode, messages::bind::BindParam, Column},
        Identifier,
    };
    use bytes::{BufMut, BytesMut};
    use chrono::NaiveDate;
    use cipherstash_client::encryption::Plaintext;
    use cipherstash_config::{ColumnConfig, ColumnMode, ColumnType};
    use postgres_types::{ToSql, Type};
    use std::ffi::CString;

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
        log::init();

        // Binary
        let val: i64 = 42;
        let mut bytes = BytesMut::with_capacity(8);
        bytes.put_i64(val);
        let param = BindParam::new(FormatCode::Binary, bytes);

        let pt = from_sql(&param, &Type::INT8).unwrap().unwrap();
        assert_eq!(pt, Plaintext::BigInt(Some(val)));

        // Text
        let val: i64 = 42;

        let binding = val.to_string();
        let bytes = binding.as_bytes();
        let bytes = BytesMut::from(bytes);

        let param = BindParam::new(FormatCode::Text, bytes);

        let pt = from_sql(&param, &Type::INT8).unwrap().unwrap();
        assert_eq!(pt, Plaintext::BigInt(Some(val)));
    }

    #[test]
    pub fn bind_param_to_plaintext_boolean() {
        log::init();

        // Binary
        let val = true;
        let mut bytes = BytesMut::with_capacity(1);
        bytes.put_u8(true as u8);
        let param = BindParam::new(FormatCode::Binary, bytes);

        let pt = from_sql(&param, &Type::BOOL).unwrap().unwrap();
        assert_eq!(pt, Plaintext::Boolean(Some(val)));

        // Text
        let val = true;

        let binding = val.to_string();
        let bytes = binding.as_bytes();
        let bytes = BytesMut::from(bytes);

        let param = BindParam::new(FormatCode::Text, bytes);

        let pt = from_sql(&param, &Type::BOOL).unwrap().unwrap();
        assert_eq!(pt, Plaintext::Boolean(Some(val)));
    }

    #[test]
    pub fn bind_param_to_plaintext_date() {
        log::init();

        // Binary
        let val = NaiveDate::parse_from_str("2025-01-01", "%Y-%m-%d").unwrap();

        let mut bytes = BytesMut::new();
        let _ = val.to_sql_checked(&Type::DATE, &mut bytes);

        let param = BindParam::new(FormatCode::Binary, bytes);

        let pt = from_sql(&param, &Type::DATE).unwrap().unwrap();
        assert_eq!(pt, Plaintext::NaiveDate(Some(val)));

        // Text
        let s = CString::new("2025-01-01").unwrap();
        let bytes = s.as_bytes_with_nul();
        let bytes = BytesMut::from(bytes);

        let param = BindParam::new(FormatCode::Text, bytes);

        let pt = from_sql(&param, &Type::DATE).unwrap().unwrap();
        assert_eq!(pt, Plaintext::NaiveDate(Some(val)));
    }
}
