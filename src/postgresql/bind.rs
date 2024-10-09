use super::{BytesMutReadString, FormatCode, NULL};
use crate::{Error, ProtocolError, SIZE_I16, SIZE_I32};
use bytes::{Buf, BufMut, BytesMut};
use std::io::Cursor;
use std::{convert::TryFrom, ffi::CString};

/// Bind (B) message.
/// See: <https://www.postgresql.org/docs/current/protocol-message-formats.html>
#[derive(Clone, Debug)]
pub struct Bind {
    pub code: char,
    #[allow(dead_code)]
    len: i64,
    pub portal: String,
    pub prepared_statement: String,
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

    pub fn len(&self) -> usize {
        self.bytes.len()
    }

    // jsonb header byte is always 1
    pub fn maybe_jsonb(&self) -> bool {
        let header = self.bytes.as_ref()[0];
        header == 1
    }

    pub fn rewrite(&mut self, bytes: &[u8]) {
        self.bytes.clear();
        self.bytes.extend_from_slice(bytes);
        self.dirty = true;
    }

    pub fn rewrite_required(&self) -> bool {
        self.dirty
    }
}

// The number of parameter format codes that follow (denoted C below).
// This can be zero to indicate that there are no parameters or that the parameters all use the default format (text);
// or one, in which case the specified format code is applied to all parameters;
// or it can equal the actual number of parameters.

impl TryFrom<&BytesMut> for Bind {
    type Error = Error;

    fn try_from(buf: &BytesMut) -> Result<Bind, Self::Error> {
        let mut cursor = Cursor::new(buf);
        let code = cursor.get_u8() as char;
        let len = cursor.get_i32();
        let portal = cursor.read_string()?;
        let prepared_statement = cursor.read_string()?;
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
            len: len as i64,
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

        let portal_binding =
            CString::new(bind.portal).map_err(|_| ProtocolError::UnexpectedNull)?;
        let portal = portal_binding.as_bytes_with_nul();

        let prepared_statement_binding =
            CString::new(bind.prepared_statement).map_err(|_| ProtocolError::UnexpectedNull)?;
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
