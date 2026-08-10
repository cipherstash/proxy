//! CipherStash diagnostic response factories.
use bytes::Bytes;
use core::fmt;
use pg_proto::{BackendMessage, DiagnosticField, DiagnosticResponse};
use regex::Regex;
use std::sync::LazyLock;
///
/// Postgres Error Codes
/// https://www.postgresql.org/docs/current/errcodes-appendix.html
pub const CODE_UNDEFINED_COLUMN: &str = "42703";
pub const CODE_INVALID_PASSWORD: &str = "28P01";
pub const CODE_RAISE_EXCEPTION: &str = "P0001";
pub const CODE_SYNTAX_ERROR: &str = "42601";
pub const CODE_INVALID_TEXT_REPRESENTATION: &str = "22P02";
pub const CODE_IDLE_SESSION_TIMEOUT: &str = "57P05";
pub const CODE_SYSTEM_ERROR: &str = "58000";

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
#[derive(Debug, Clone, PartialEq)]
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
    pub fn into_backend_message(self) -> BackendMessage {
        BackendMessage::ErrorResponse(self.into())
    }
    /// Create a FATAL error response for connection timeout.
    ///
    /// Uses PostgreSQL error code 57P05 (idle_session_timeout). While this code
    /// is technically for idle session timeouts, it is the closest match for a
    /// proxy-enforced connection timeout. The alternative 08006 (connection_failure)
    /// implies a network-level failure, which is misleading — the proxy is
    /// deliberately terminating a connection that exceeded its time limit.
    pub fn connection_timeout(message: String) -> Self {
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
                    value: CODE_IDLE_SESSION_TIMEOUT.to_string(),
                },
                Field {
                    code: ErrorResponseCode::Message,
                    value: message,
                },
            ],
        }
    }

    /// Whether this error carries FATAL severity — the client abandons the
    /// connection on receipt.
    pub fn is_fatal(&self) -> bool {
        self.fields
            .iter()
            .any(|field| field.code == ErrorResponseCode::Severity && field.value == "FATAL")
    }

    pub fn invalid_password(message: String) -> Self {
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
                    value: CODE_INVALID_PASSWORD.to_string(),
                },
                Field {
                    code: ErrorResponseCode::Message,
                    value: message,
                },
            ],
        }
    }

    ///
    /// SQL Parse error as PostgreSQL error
    /// Code: 42601 syntax_errpr
    ///
    /// As EncryptError is an enum, this can be passed a different error variation.
    ///
    pub fn invalid_sql_statement(message: String) -> Self {
        let line = extract_line_from_parse_error(&message);
        let position: Option<usize> = extract_position_from_parse_error(&message);

        let mut fields = vec![
            Field {
                code: ErrorResponseCode::Severity,
                value: "ERROR".to_string(),
            },
            Field {
                code: ErrorResponseCode::SeverityLegacy,
                value: "ERROR".to_string(),
            },
            Field {
                code: ErrorResponseCode::Code,
                value: CODE_SYNTAX_ERROR.to_string(),
            },
            Field {
                code: ErrorResponseCode::Message,
                value: message,
            },
        ];

        if let Some(line) = line {
            fields.push(Field {
                code: ErrorResponseCode::Line,
                value: line.to_string(),
            });
        }
        if let Some(position) = position {
            fields.push(Field {
                code: ErrorResponseCode::Position,
                value: position.to_string(),
            });
        }

        Self { fields }
    }

    ///
    /// Invalid parameter as PostgreSQL error
    /// Code: 22P02 invalid_text_representation
    ///
    /// As EncryptError is an enum, this can be passed a different error variation.
    ///
    pub fn invalid_parameter(message: String, table: &str, column: &str) -> Self {
        Self {
            fields: vec![
                Field {
                    code: ErrorResponseCode::Severity,
                    value: "ERROR".to_string(),
                },
                Field {
                    code: ErrorResponseCode::SeverityLegacy,
                    value: "ERROR".to_string(),
                },
                Field {
                    code: ErrorResponseCode::Code,
                    value: CODE_INVALID_TEXT_REPRESENTATION.to_string(),
                },
                Field {
                    code: ErrorResponseCode::Message,
                    value: message,
                },
                // TODO: make this work more good
                // URL is curently in message, so this looks like a bug atm
                // Field {
                //     code: ErrorResponseCode::Detail,
                //     value: ERROR_DOC_ENCRYPT_INVALID_PARAMETER_URL.to_string(),
                // },
                Field {
                    code: ErrorResponseCode::Table,
                    value: table.to_string(),
                },
                Field {
                    code: ErrorResponseCode::Column,
                    value: column.to_string(),
                },
            ],
        }
    }

    ///
    /// Unknown encrypted column as PostgreSQL error
    /// Code: 42703 undefined_column
    ///
    pub fn unknown_column(message: String, table: &str, column: &str) -> Self {
        Self {
            fields: vec![
                Field {
                    code: ErrorResponseCode::Severity,
                    value: "ERROR".to_string(),
                },
                Field {
                    code: ErrorResponseCode::SeverityLegacy,
                    value: "ERROR".to_string(),
                },
                Field {
                    code: ErrorResponseCode::Code,
                    value: CODE_UNDEFINED_COLUMN.to_string(),
                },
                Field {
                    code: ErrorResponseCode::Message,
                    value: message,
                },
                // TODO: make this work more good
                // URL is curently in message, so this looks like a bug atm
                // Field {
                //     code: ErrorResponseCode::Detail,
                //     value: ERROR_DOC_ENCRYPT_UNKNOWN_COLUMN_URL.to_string(),
                // },
                Field {
                    code: ErrorResponseCode::Table,
                    value: table.to_string(),
                },
                Field {
                    code: ErrorResponseCode::Column,
                    value: column.to_string(),
                },
                Field {
                    code: ErrorResponseCode::Routine,
                    value: "cipherstash-proxy".to_string(),
                },
            ],
        }
    }

    pub fn system_error(message: String) -> Self {
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
                    value: CODE_SYSTEM_ERROR.to_string(),
                },
                Field {
                    code: ErrorResponseCode::Message,
                    value: message,
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

impl From<&DiagnosticResponse> for ErrorResponse {
    fn from(response: &DiagnosticResponse) -> Self {
        Self {
            fields: response
                .fields
                .iter()
                .map(|field| Field {
                    code: field.code.into(),
                    value: String::from_utf8_lossy(&field.value).into_owned(),
                })
                .collect(),
        }
    }
}

impl From<ErrorResponse> for DiagnosticResponse {
    fn from(response: ErrorResponse) -> Self {
        Self {
            fields: response
                .fields
                .into_iter()
                .map(|field| DiagnosticField {
                    code: field.code.into(),
                    value: Bytes::from(field.value),
                })
                .collect(),
        }
    }
}

///
/// Extracts line (if present) from a SQL Parser error message
///
fn extract_line_from_parse_error(error_message: &str) -> Option<usize> {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*Line:\s*(\d+)").unwrap());
    RE.captures(error_message)
        .and_then(|c| c.get(1)?.as_str().parse::<usize>().ok())
}

///
/// Extracts position (if present) from a SQL Parser error message
/// Column in the error message is the "text" column, not a reference to a database column
///
fn extract_position_from_parse_error(error_message: &str) -> Option<usize> {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*Column:\s*(\d+)").unwrap());

    RE.captures(error_message)
        .and_then(|c| c.get(1)?.as_str().parse::<usize>().ok())
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
    use super::ErrorResponseCode;
    use crate::postgresql::diagnostics::ErrorResponse;
    use bytes::Bytes;
    use pg_proto::{BackendMessage, DiagnosticField, DiagnosticResponse};

    #[test]
    pub fn parse_error_response_message() {
        let response = DiagnosticResponse {
            fields: vec![
                (b'S', "ERROR"),
                (b'V', "ERROR"),
                (b'C', "26000"),
                (b'M', "prepared statement \"a37\" does not exist"),
                (b'F', "prepare.c"),
                (b'L', "454"),
                (b'R', "FetchPreparedStatement"),
            ]
            .into_iter()
            .map(|(code, value)| DiagnosticField {
                code,
                value: Bytes::from(value),
            })
            .collect(),
        };
        let error_response = ErrorResponse::from(&response);
        assert_eq!(error_response.fields.len(), 7);
        let BackendMessage::ErrorResponse(round_trip) = error_response.into_backend_message()
        else {
            panic!("expected ErrorResponse")
        };
        assert_eq!(round_trip, response);
    }

    #[test]
    pub fn sql_parse_error_response() {
        let response = ErrorResponse::invalid_sql_statement(
            "sql syntax error in blah vtha Line: 1, Column: 2".to_string(),
        );

        let line = response
            .fields
            .iter()
            .find(|f| f.code == ErrorResponseCode::Line)
            .unwrap();

        assert_eq!(line.value, "1".to_string());

        let position = response
            .fields
            .iter()
            .find(|f| f.code == ErrorResponseCode::Position)
            .unwrap();

        assert_eq!(position.value, "2".to_string());

        let response = ErrorResponse::invalid_sql_statement(
            "sql syntax error in blah vtha Column: 2".to_string(),
        );

        let line = response
            .fields
            .iter()
            .find(|f| f.code == ErrorResponseCode::Line);

        assert!(line.is_none());

        let position = response
            .fields
            .iter()
            .find(|f| f.code == ErrorResponseCode::Position)
            .unwrap();

        assert_eq!(position.value, "2".to_string());
    }
}
