use crate::error::EncryptError;
use crate::{error::Error, postgresql::format_code::FormatCode};
use bigdecimal::FromPrimitive;
use bytes::BytesMut;
use cipherstash_client::encryption::Plaintext;
use postgres_types::ToSql;
use postgres_types::Type;
use rust_decimal::Decimal;

pub fn to_sql(plaintext: &Plaintext, format_code: &FormatCode) -> Result<Option<BytesMut>, Error> {
    let bytes = match format_code {
        FormatCode::Text => text_to_sql(plaintext)?,
        FormatCode::Binary => binary_to_sql(plaintext)?,
    };

    Ok(Some(bytes))
}

fn text_to_sql(plaintext: &Plaintext) -> Result<BytesMut, Error> {
    let s = match &plaintext {
        Plaintext::Utf8Str(Some(x)) => x.to_string(),
        Plaintext::Int(Some(x)) => x.to_string(),
        Plaintext::BigInt(Some(x)) => x.to_string(),
        Plaintext::BigUInt(Some(x)) => x.to_string(),
        Plaintext::Boolean(Some(x)) => x.to_string(),
        Plaintext::Decimal(Some(x)) => x.to_string(),
        Plaintext::Float(Some(x)) => x.to_string(),
        Plaintext::NaiveDate(Some(x)) => x.to_string(),
        Plaintext::SmallInt(Some(x)) => x.to_string(),
        Plaintext::Timestamp(Some(x)) => x.to_string(),
        Plaintext::JsonB(Some(x)) => x.to_string(),
        _ => "".to_string(),
    };

    Ok(BytesMut::from(s.as_bytes()))
}

fn binary_to_sql(plaintext: &Plaintext) -> Result<BytesMut, Error> {
    let mut bytes = BytesMut::new();

    let result = match &plaintext {
        Plaintext::BigInt(x) => x.to_sql_checked(&Type::INT8, &mut bytes),
        Plaintext::Boolean(x) => x.to_sql_checked(&Type::BOOL, &mut bytes),
        Plaintext::Float(x) => x.to_sql_checked(&Type::FLOAT8, &mut bytes),
        Plaintext::Int(x) => x.to_sql_checked(&Type::INT4, &mut bytes),
        Plaintext::NaiveDate(x) => x.to_sql_checked(&Type::DATE, &mut bytes),
        Plaintext::SmallInt(x) => x.to_sql_checked(&Type::INT2, &mut bytes),
        Plaintext::Timestamp(x) => x.to_sql_checked(&Type::TIMESTAMPTZ, &mut bytes),
        Plaintext::Utf8Str(x) => x.to_sql_checked(&Type::TEXT, &mut bytes),
        Plaintext::JsonB(x) => x.to_sql_checked(&Type::JSONB, &mut bytes),
        Plaintext::Decimal(x) => x.to_sql_checked(&Type::NUMERIC, &mut bytes),
        Plaintext::BigUInt(x) => {
            let d = x.map(|x| Decimal::from_u64(x)).flatten();
            d.to_sql_checked(&Type::NUMERIC, &mut bytes)
        }
    };

    match result {
        Ok(_) => Ok(bytes),
        Err(_e) => Err(EncryptError::PlaintextCouldNotBeEncoded.into()),
    }
}
