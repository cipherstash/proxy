use super::{maybe_json, maybe_jsonb, Name, NULL};
use crate::eql;
use crate::error::{Error, MappingError, ProtocolError};
use crate::log::MAPPER;
use crate::postgresql::context::column::Column;
use crate::postgresql::data::bind_param_from_sql;
use crate::postgresql::format_code::FormatCode;
use crate::postgresql::protocol::BytesMutReadString;
use crate::{SIZE_I16, SIZE_I32};
use bytes::{Buf, BufMut, BytesMut};
use cipherstash_client::encryption::Plaintext;
use eql_mapper::EqlTermVariant;
use postgres_types::Type;
use std::fmt::{self, Display, Formatter};
use std::io::Cursor;
use std::{convert::TryFrom, ffi::CString};
use tracing::debug;

/// Bind (B) message.
/// See: <https://www.postgresql.org/docs/current/protocol-message-formats.html>
#[derive(Clone, Debug)]
pub struct Bind {
    pub code: char,
    pub portal: Name,
    pub prepared_statement: Name,
    pub num_param_format_codes: i16,
    pub param_format_codes: Vec<FormatCode>,
    pub num_param_values: i16,
    pub param_values: Vec<BindParam>,
    pub num_result_column_format_codes: i16,
    pub result_columns_format_codes: Vec<FormatCode>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct BindParam {
    pub format_code: FormatCode,
    pub bytes: BytesMut,
    dirty: bool,
}

impl Bind {
    pub fn requires_rewrite(&self) -> bool {
        self.param_values
            .iter()
            .any(|param| param.requires_rewrite())
    }

    pub fn to_plaintext(
        &self,
        param_columns: &[Option<Column>],
        param_types: &[i32],
    ) -> Result<Vec<Option<Plaintext>>, Error> {
        let plaintexts = self
            .param_values
            .iter()
            .zip(param_columns.iter())
            .enumerate()
            .map(|(idx, (param, col))| match col {
                Some(col) => {
                    let bound_param_type = get_param_type(idx, param_types, col);

                    debug!(
                        target: MAPPER,
                        col = ?col, bound_param_type = ?bound_param_type
                    );

                    // Convert param bytes into a Plaintext wrapping a Value
                    // If the param type is different, will convert the bound type to the correct Plaintext variant identified by the cast_type
                    let plaintext = bind_param_from_sql(
                        param,
                        &bound_param_type,
                        EqlTermVariant::Full,
                        col.cast_type(),
                    )
                    .map_err(|_| MappingError::InvalidParameter(col.to_owned()))?;

                    Ok(plaintext)
                }
                None => Ok(None),
            })
            .collect::<Result<Vec<_>, Error>>()?;
        Ok(plaintexts)
    }

    pub fn rewrite(&mut self, encrypted: Vec<Option<eql::EqlEncrypted>>) -> Result<(), Error> {
        for (idx, ct) in encrypted.iter().enumerate() {
            if let Some(ct) = ct {
                let json = serde_json::to_value(ct)?;

                // convert json to bytes
                let bytes = json.to_string().into_bytes();

                self.param_values[idx].rewrite(&bytes);
            }
        }
        Ok(())
    }
}

///
/// Param type is either provided with Parse message or the column type
/// Column type is the cast of the encrypted column
///
fn get_param_type(idx: usize, param_types: &[i32], col: &Column) -> Type {
    param_types
        .get(idx)
        .and_then(|oid| Type::from_oid(*oid as u32))
        .map_or_else(|| col.postgres_type.clone(), |t| t)
}

impl BindParam {
    pub fn new(format_code: FormatCode, bytes: BytesMut) -> Self {
        Self {
            format_code,
            bytes,
            dirty: false,
        }
    }

    pub fn null() -> Self {
        Self {
            format_code: FormatCode::Text,
            bytes: BytesMut::new(),
            dirty: false,
        }
    }

    ///
    /// Returns the actual length of the param bytes
    /// The actual byte length needs to be used when calculating the Bind message length
    /// If NULL returns 0 as the actual byte length
    /// Not to be confused with the *param* len as encoded in the Bind message
    ///
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }

    ///
    /// Returns the length of the param for representation in the Bind message
    /// If NULL returns -1 as required by the PostgreSQL protocol
    ///
    pub fn len(&self) -> i32 {
        if self.is_null() {
            return NULL;
        }
        self.bytes.len() as i32
    }

    pub fn rewrite(&mut self, bytes: &[u8]) {
        self.bytes.clear();

        if self.is_binary() {
            self.bytes.put_u8(1);
        }

        self.bytes.extend_from_slice(bytes);
        self.dirty = true;
    }

    pub fn requires_rewrite(&self) -> bool {
        self.dirty
    }

    pub fn maybe_plaintext(&self) -> bool {
        self.is_text() && maybe_json(&self.bytes) || self.is_binary() && maybe_jsonb(&self.bytes)
    }

    ///
    /// If the text format is binary, returns a reference to the bytes without the jsonb header byte
    ///
    pub fn json_bytes(&self) -> &[u8] {
        if self.is_binary() {
            &self.bytes[1..]
        } else {
            &self.bytes[0..]
        }
    }

    pub fn is_null(&self) -> bool {
        self.bytes.is_empty()
    }

    pub fn is_text(&self) -> bool {
        self.format_code == FormatCode::Text
    }

    pub fn is_binary(&self) -> bool {
        self.format_code == FormatCode::Binary
    }
}

impl Display for BindParam {
    fn fmt(&self, f: &mut Formatter) -> fmt::Result {
        let s = String::from_utf8_lossy(&self.bytes).to_string();
        write!(f, "{s}")
    }
}

impl From<&BindParam> for Option<eql::Plaintext> {
    fn from(bind_param: &BindParam) -> Self {
        if !bind_param.maybe_plaintext() {
            return None;
        }

        let bytes = bind_param.json_bytes();
        let s = std::str::from_utf8(bytes).unwrap_or("");

        match serde_json::from_str(s) {
            Ok(pt) => Some(pt),
            Err(e) => {
                debug!(
                    param = s,
                    error = e.to_string(),
                    "Failed to parse parameter"
                );
                None
            }
        }
    }
}

impl TryFrom<&BytesMut> for Bind {
    type Error = Error;

    fn try_from(buf: &BytesMut) -> Result<Bind, Self::Error> {
        let mut cursor = Cursor::new(buf);
        let code = cursor.get_u8() as char;
        let _len = cursor.get_i32();

        let portal = cursor.read_string()?;
        let portal = Name(portal);

        let prepared_statement = cursor.read_string()?;
        let prepared_statement = Name(prepared_statement);

        let num_param_format_codes = cursor.get_i16();
        let mut param_format_codes = Vec::new();

        for _ in 0..num_param_format_codes {
            param_format_codes.push(cursor.get_i16().into());
        }

        let num_param_values = cursor.get_i16();
        let mut param_values = Vec::new();

        for idx in 0..num_param_values as usize {
            let param_len = cursor.get_i32();

            let format_code = match num_param_format_codes {
                0 => FormatCode::Text,
                1 => param_format_codes[0],
                _ => param_format_codes[idx],
            };

            // NULL parameters have a length of -1 and no bytes
            match param_len {
                NULL => {
                    param_values.push(BindParam::null());
                }
                _ => {
                    let mut bytes = BytesMut::with_capacity(param_len as usize);
                    bytes.resize(param_len as usize, b'0');
                    cursor.copy_to_slice(&mut bytes);
                    param_values.push(BindParam::new(format_code, bytes));
                }
            }
        }

        let num_result_column_format_codes = cursor.get_i16();
        let mut result_columns_format_codes = Vec::new();

        for _ in 0..num_result_column_format_codes {
            result_columns_format_codes.push(cursor.get_i16().into());
        }

        Ok(Bind {
            code,
            portal,
            prepared_statement,
            num_param_format_codes,
            param_format_codes,
            num_param_values,
            param_values,
            num_result_column_format_codes,
            result_columns_format_codes,
        })
    }
}

impl TryFrom<Bind> for BytesMut {
    type Error = Error;

    fn try_from(bind: Bind) -> Result<BytesMut, Self::Error> {
        let mut bytes = BytesMut::new();

        let portal_binding = CString::new(&*bind.portal)?;
        let portal = portal_binding.as_bytes_with_nul();

        let prepared_statement_binding = CString::new(&*bind.prepared_statement)?;
        let prepared_statement = prepared_statement_binding.as_bytes_with_nul();

        if bind.num_param_format_codes != bind.param_format_codes.len() as i16 {
            let err = ProtocolError::ParameterFormatCodesMismatch {
                expected: bind.num_param_format_codes as usize,
                received: bind.param_format_codes.len(),
            };
            return Err(err.into());
        }

        if bind.num_result_column_format_codes != bind.result_columns_format_codes.len() as i16 {
            let err = ProtocolError::ParameterResultFormatCodesMismatch {
                expected: bind.num_result_column_format_codes as usize,
                received: bind.result_columns_format_codes.len(),
            };
            return Err(err.into());
        }

        // sum of param byte_lens (the *actual* byte lengths of the parameters)
        let param_byte_len = &bind
            .param_values
            .iter()
            .fold(0, |acc, param| acc + SIZE_I32 + param.byte_len());

        let len = SIZE_I32 // self/len of len
            + portal.len()
            + prepared_statement.len()
            + SIZE_I16 // num_param_format_codes
            + SIZE_I16 * bind.num_param_format_codes as usize // num_param_format_codes
            + SIZE_I16  // num_param_values
            + param_byte_len // parameter bytes
            + SIZE_I16 // num_result_column_format_codes
            + SIZE_I16 * bind.num_result_column_format_codes as usize;

        bytes.put_u8(bind.code as u8);
        bytes.put_i32(len as i32);
        bytes.put_slice(portal);
        bytes.put_slice(prepared_statement);
        bytes.put_i16(bind.num_param_format_codes);
        for param_format_code in bind.param_format_codes {
            bytes.put_i16(param_format_code.into());
        }

        let num_param_values = bind.param_values.len() as i16;
        bytes.put_i16(num_param_values);

        for p in bind.param_values {
            // len is not the same as byte_len
            // A NULL param len is -1
            bytes.put_i32(p.len());
            bytes.put_slice(&p.bytes);
        }

        bytes.put_i16(bind.num_result_column_format_codes);
        for result_column_format_code in bind.result_columns_format_codes {
            bytes.put_i16(result_column_format_code.into());
        }

        Ok(bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::BindParam;
    use crate::{
        config::LogConfig,
        log,
        postgresql::{format_code::FormatCode, messages::bind::Bind},
    };
    use bytes::BytesMut;

    fn to_message(s: &[u8]) -> BytesMut {
        BytesMut::from(s)
    }

    #[test]
    pub fn parse_bind() {
        log::init(LogConfig::default());
        let bytes =
            to_message(b"B\0\0\0\x18\0\0\0\x01\0\x01\0\x01\0\0\0\x04.\xbe\x8a\xd4\0\x01\0\x01");

        let expected = bytes.clone();

        let bind = Bind::try_from(&bytes).unwrap();

        assert_eq!(bind.param_values.len(), 1);
        assert_eq!(bind.result_columns_format_codes[0], FormatCode::Binary);

        let bytes = BytesMut::try_from(bind).unwrap();
        assert_eq!(bytes, expected);
    }

    #[test]
    pub fn parse_bind_with_null_param() {
        log::init(LogConfig::default());

        // Bind message from statement INSERT INTO encrypted (id, plaintext, plaintext_date, encrypted_text) VALUES ($1, $2, $3, $4)
        let bytes =
            to_message(b"B\0\0\0N\0s0\0\0\x04\0\x01\0\x01\0\x01\0\x01\0\x04\0\0\0\x084\xd8\x1d@\x83U\x0em\0\0\0\tplaintext\xff\xff\xff\xff\0\0\0\x15hello@cipherstash.com\0\x01\0\x01");

        let expected = bytes.clone();

        let bind = Bind::try_from(&bytes).unwrap();

        assert_eq!(bind.param_values.len(), 4);

        let bytes = BytesMut::try_from(bind).unwrap();
        assert_eq!(bytes, expected);
    }

    #[test]
    fn bind_should_rewrite() {
        log::init(LogConfig::default());

        let bytes = "hello".into();
        let mut param = BindParam::new(FormatCode::Text, bytes);

        param.rewrite("world".as_bytes());

        assert!(param.requires_rewrite());
    }
}
