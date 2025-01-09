use super::{maybe_json, maybe_jsonb, Destination, NULL};
use crate::eql;
use crate::error::{Error, ProtocolError};
use crate::postgresql::format_code::FormatCode;
use crate::postgresql::protocol::BytesMutReadString;
use crate::{SIZE_I16, SIZE_I32};
use bytes::{Buf, BufMut, BytesMut};
use std::io::{BufRead, Cursor};
use std::{convert::TryFrom, ffi::CString};
use tracing::debug;

/// Bind (B) message.
/// See: <https://www.postgresql.org/docs/current/protocol-message-formats.html>
#[derive(Clone, Debug)]
pub struct Bind {
    pub code: char,
    pub portal: Destination,
    pub prepared_statement: Destination,
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
    pub fn should_rewrite(&self) -> bool {
        self.param_values.iter().any(|param| param.should_rewrite())
    }

    pub fn to_plaintext(&self) -> Result<Vec<Option<eql::Plaintext>>, Error> {
        Ok(self.param_values.iter().map(|param| param.into()).collect())
    }

    pub fn rewrite(&mut self, encrypted: Vec<Option<eql::Ciphertext>>) -> Result<(), Error> {
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

    pub fn as_string(&self) -> Result<String, Error> {
        let mut cursor = Cursor::new(&self.bytes);
        let mut buf = Vec::with_capacity(512);
        match cursor.read_until(b'\0', &mut buf) {
            Ok(_) => Ok(String::from_utf8_lossy(&buf[..buf.len() - 1]).to_string()),
            Err(err) => Err(err.into()),
        }
    }

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    pub fn rewrite(&mut self, bytes: &[u8]) {
        self.bytes.clear();

        if self.is_binary() {
            self.bytes.put_u8(1);
        }

        self.bytes.extend_from_slice(bytes);
        self.dirty = true;
    }

    pub fn should_rewrite(&self) -> bool {
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
        let portal = Destination::new(portal);

        let prepared_statement = cursor.read_string()?;
        let prepared_statement = Destination::new(prepared_statement);

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

        let portal_binding = CString::new(bind.portal.as_str())?;
        let portal = portal_binding.as_bytes_with_nul();

        let prepared_statement_binding = CString::new(bind.prepared_statement.as_str())?;
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

        let param_len = &bind
            .param_values
            .iter()
            .fold(0, |acc, param| acc + SIZE_I32 + param.len());

        let len = SIZE_I32 // self
            + portal.len()
            + prepared_statement.len()
            + SIZE_I16 // num_param_format_codes
            + SIZE_I16 * bind.num_param_format_codes as usize // num_param_format_codes
            + SIZE_I16
            + param_len // num_param_values
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
            bytes.put_i32(p.len() as i32);
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
    use crate::{log, postgresql::format_code::FormatCode};

    use super::BindParam;

    #[test]
    fn bind_should_rewrite() {
        log::init();

        let bytes = "hello".into();
        let mut param = BindParam::new(FormatCode::Text, bytes);

        param.rewrite("world".as_bytes());

        assert!(param.should_rewrite());
    }
}
