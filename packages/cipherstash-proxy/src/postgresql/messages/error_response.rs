use super::BackendCode;
use crate::error::{Error, ProtocolError};
use crate::postgresql::protocol::BytesMutReadString;
use crate::SIZE_I32;
use bytes::{Buf, BufMut, BytesMut};
use core::fmt;
use std::io::Cursor;
use std::{convert::TryFrom, ffi::CString};

///
/// ErrorResponse (B)
/// https://www.postgresql.org/docs/current/protocol-message-formats.html#PROTOCOL-MESSAGE-FORMATS-ERRORRESPONSE
///
#[derive(Debug, Clone)]
pub struct ErrorResponse {
    pub fields: Vec<Field>,
}

#[derive(Debug, Clone)]
pub struct Field {
    pub code: ErrorResponseCode,
    pub value: String,
}

/// ErrorResponseCodes
/// https://www.postgresql.org/docs/current/protocol-error-fields.html
#[derive(Debug, Clone)]
pub enum ErrorResponseCode {
    Severity,
    SeverityLegacy,
    Code,
    Message,
    Detail,
    Hint,
    Position,
    InternalPosition,
    InternalQuery,
    Where,
    Schema,
    Table,
    Column,
    DataType,
    Constraint,
    File,
    Line,
    Routine,
    Unknown(char),
}

impl ErrorResponse {
    pub fn invalid_password(username: &str) -> Self {
        Self {
            fields: vec![
                Field {
                    code: ErrorResponseCode::Severity,
                    value: "FATAL".to_string(),
                },
                Field {
                    code: ErrorResponseCode::SeverityLegacy,
                    value: "FATAL".to_string(),
                },
                Field {
                    code: ErrorResponseCode::Code,
                    value: "28P01".to_string(),
                },
                Field {
                    code: ErrorResponseCode::Message,
                    value: format!("password authentication failed for user \"{}\"", username),
                },
            ],
        }
    }

    pub fn tls_required() -> Self {
        Self {
            fields: vec![
                Field {
                    code: ErrorResponseCode::Severity,
                    value: "FATAL".to_string(),
                },
                Field {
                    code: ErrorResponseCode::SeverityLegacy,
                    value: "FATAL".to_string(),
                },
                Field {
                    code: ErrorResponseCode::Code,
                    value: "08001".to_string(),
                },
                Field {
                    code: ErrorResponseCode::Message,
                    value: "Transport Layer Security (TLS) connection is required".to_string(),
                },
            ],
        }
    }
}

impl TryFrom<&BytesMut> for ErrorResponse {
    type Error = Error;

    fn try_from(buf: &BytesMut) -> Result<ErrorResponse, Error> {
        let mut cursor = Cursor::new(buf);
        let code = cursor.get_u8();

        if BackendCode::from(code) != BackendCode::ErrorResponse {
            return Err(ProtocolError::UnexpectedMessageCode {
                expected: BackendCode::ErrorResponse.into(),
                received: code as char,
            }
            .into());
        }

        let _len = cursor.get_i32();

        // The message body consists of one or more identified fields, followed by a zero byte as a terminator.
        let mut fields = Vec::new();

        loop {
            let code = cursor.get_u8();

            // zero byte is terminator
            if code == 0 {
                break;
            }

            let value = cursor.read_string()?;
            let field = Field {
                code: code.into(),
                value,
            };
            fields.push(field);
        }

        Ok(ErrorResponse { fields })
    }
}

impl TryFrom<ErrorResponse> for BytesMut {
    type Error = Error;

    fn try_from(error_response: ErrorResponse) -> Result<BytesMut, Error> {
        let mut field_bytes = BytesMut::new();

        for field in error_response.fields {
            let value = CString::new(field.value)?;
            let value = value.as_bytes_with_nul();

            field_bytes.put_u8(field.code.into());
            field_bytes.put_slice(value);
        }
        field_bytes.put_u8(0); // field terminator

        let mut bytes = BytesMut::new();

        let len = SIZE_I32 + field_bytes.len(); // len + fields

        bytes.put_u8(BackendCode::ErrorResponse.into());
        bytes.put_i32(len as i32);
        bytes.put_slice(&field_bytes);

        Ok(bytes)
    }
}

impl fmt::Display for ErrorResponse {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        for field in self.fields.iter() {
            let msg = match field.code {
                ErrorResponseCode::Severity => "Severity",
                ErrorResponseCode::SeverityLegacy => continue, // skipped, always appears with `S` in versions we support
                ErrorResponseCode::Code => "Code",
                ErrorResponseCode::Message => "Message",
                ErrorResponseCode::Detail => "Detail",
                ErrorResponseCode::Hint => "Hint",
                ErrorResponseCode::Position => "Position",
                ErrorResponseCode::InternalPosition => "Internal Position",
                ErrorResponseCode::InternalQuery => "Internal Query",
                ErrorResponseCode::Where => "Where",
                ErrorResponseCode::Schema => "Schema",
                ErrorResponseCode::Table => "Table",
                ErrorResponseCode::Column => "Column",
                ErrorResponseCode::DataType => "Data Type",
                ErrorResponseCode::Constraint => "Constraint",
                ErrorResponseCode::File => "File",
                ErrorResponseCode::Line => "Line",
                ErrorResponseCode::Routine => "Routine",
                ErrorResponseCode::Unknown(_) => "Unknown",
            };
            write!(f, "{} ({}): {} ", msg, char::from(&field.code), field.value)?;
        }

        Ok(())
    }
}

impl From<ErrorResponseCode> for u8 {
    fn from(code: ErrorResponseCode) -> Self {
        match code {
            ErrorResponseCode::Severity => b'S',
            ErrorResponseCode::SeverityLegacy => b'V',
            ErrorResponseCode::Code => b'C',
            ErrorResponseCode::Message => b'M',
            ErrorResponseCode::Detail => b'D',
            ErrorResponseCode::Hint => b'H',
            ErrorResponseCode::Position => b'P',
            ErrorResponseCode::InternalPosition => b'p',
            ErrorResponseCode::InternalQuery => b'q',
            ErrorResponseCode::Where => b'W',
            ErrorResponseCode::Schema => b's',
            ErrorResponseCode::Table => b't',
            ErrorResponseCode::Column => b'c',
            ErrorResponseCode::DataType => b'd',
            ErrorResponseCode::Constraint => b'n',
            ErrorResponseCode::File => b'F',
            ErrorResponseCode::Line => b'L',
            ErrorResponseCode::Routine => b'R',
            ErrorResponseCode::Unknown(c) => c as u8,
        }
    }
}

impl From<&ErrorResponseCode> for char {
    fn from(code: &ErrorResponseCode) -> Self {
        match code {
            ErrorResponseCode::Severity => 'S',
            ErrorResponseCode::SeverityLegacy => 'V',
            ErrorResponseCode::Code => 'C',
            ErrorResponseCode::Message => 'M',
            ErrorResponseCode::Detail => 'D',
            ErrorResponseCode::Hint => 'H',
            ErrorResponseCode::Position => 'P',
            ErrorResponseCode::InternalPosition => 'p',
            ErrorResponseCode::InternalQuery => 'q',
            ErrorResponseCode::Where => 'W',
            ErrorResponseCode::Schema => 's',
            ErrorResponseCode::Table => 't',
            ErrorResponseCode::Column => 'c',
            ErrorResponseCode::DataType => 'd',
            ErrorResponseCode::Constraint => 'n',
            ErrorResponseCode::File => 'F',
            ErrorResponseCode::Line => 'L',
            ErrorResponseCode::Routine => 'R',
            ErrorResponseCode::Unknown(c) => c.to_owned(),
        }
    }
}

impl From<u8> for ErrorResponseCode {
    fn from(byte: u8) -> Self {
        match byte {
            b'S' => ErrorResponseCode::Severity,
            b'V' => ErrorResponseCode::SeverityLegacy,
            b'C' => ErrorResponseCode::Code,
            b'M' => ErrorResponseCode::Message,
            b'D' => ErrorResponseCode::Detail,
            b'H' => ErrorResponseCode::Hint,
            b'P' => ErrorResponseCode::Position,
            b'p' => ErrorResponseCode::InternalPosition,
            b'q' => ErrorResponseCode::InternalQuery,
            b'W' => ErrorResponseCode::Where,
            b's' => ErrorResponseCode::Schema,
            b't' => ErrorResponseCode::Table,
            b'c' => ErrorResponseCode::Column,
            b'd' => ErrorResponseCode::DataType,
            b'n' => ErrorResponseCode::Constraint,
            b'F' => ErrorResponseCode::File,
            b'L' => ErrorResponseCode::Line,
            b'R' => ErrorResponseCode::Routine,
            c => ErrorResponseCode::Unknown(c as char),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::postgresql::messages::error_response::ErrorResponse;

    use bytes::BytesMut;

    fn to_message(s: &[u8]) -> BytesMut {
        BytesMut::from(s)
    }

    #[test]
    pub fn parse_error_response_message() {
        let message = to_message(b"E\0\0\0kSERROR\0VERROR\0C26000\0Mprepared statement \"a37\" does not exist\0Fprepare.c\0L454\0RFetchPreparedStatement\0\0Z\0\0\0\x05I");

        let error_response = ErrorResponse::try_from(&message).expect("ok");
        assert_eq!(error_response.fields.len(), 7);

        // let next = cursor.get_u8() as char;
        // assert_eq!(next, 'Z');

        let bytes = BytesMut::try_from(error_response).expect("ok");
        let message = to_message(b"E\0\0\0kSERROR\0VERROR\0C26000\0Mprepared statement \"a37\" does not exist\0Fprepare.c\0L454\0RFetchPreparedStatement\0\0");
        assert_eq!(bytes, message);
    }
}
