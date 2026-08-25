//! CipherStash diagnostic response policy over pg-proto's wire model.
use bytes::Bytes;
use pg_proto::{DiagnosticField, DiagnosticResponse};
use regex::Regex;
use std::sync::LazyLock;

pub const CODE_UNDEFINED_COLUMN: &str = "42703";
pub const CODE_INVALID_PASSWORD: &str = "28P01";
pub const CODE_RAISE_EXCEPTION: &str = "P0001";
pub const CODE_SYNTAX_ERROR: &str = "42601";
pub const CODE_INVALID_TEXT_REPRESENTATION: &str = "22P02";
pub const CODE_IDLE_SESSION_TIMEOUT: &str = "57P05";
pub const CODE_SYSTEM_ERROR: &str = "58000";

fn response(fields: impl IntoIterator<Item = (u8, String)>) -> DiagnosticResponse {
    DiagnosticResponse {
        fields: fields
            .into_iter()
            .map(|(code, value)| DiagnosticField {
                code,
                value: Bytes::from(value),
            })
            .collect(),
    }
}

fn standard(severity: &str, code: &str, message: String) -> DiagnosticResponse {
    response([
        (b'S', severity.to_owned()),
        (b'V', severity.to_owned()),
        (b'C', code.to_owned()),
        (b'M', message),
    ])
}

pub fn connection_timeout(message: String) -> DiagnosticResponse {
    standard("FATAL", CODE_IDLE_SESSION_TIMEOUT, message)
}

pub fn invalid_password(message: String) -> DiagnosticResponse {
    standard("FATAL", CODE_INVALID_PASSWORD, message)
}

pub fn invalid_sql_statement(message: String) -> DiagnosticResponse {
    let line = extract_line_from_parse_error(&message);
    let position = extract_position_from_parse_error(&message);
    let mut fields = vec![
        (b'S', "ERROR".to_owned()),
        (b'V', "ERROR".to_owned()),
        (b'C', CODE_SYNTAX_ERROR.to_owned()),
        (b'M', message),
    ];
    if let Some(line) = line {
        fields.push((b'L', line.to_string()));
    }
    if let Some(position) = position {
        fields.push((b'P', position.to_string()));
    }
    response(fields)
}

pub fn invalid_parameter(message: String, table: &str, column: &str) -> DiagnosticResponse {
    response([
        (b'S', "ERROR".to_owned()),
        (b'V', "ERROR".to_owned()),
        (b'C', CODE_INVALID_TEXT_REPRESENTATION.to_owned()),
        (b'M', message),
        (b't', table.to_owned()),
        (b'c', column.to_owned()),
    ])
}

pub fn unknown_column(message: String, table: &str, column: &str) -> DiagnosticResponse {
    response([
        (b'S', "ERROR".to_owned()),
        (b'V', "ERROR".to_owned()),
        (b'C', CODE_UNDEFINED_COLUMN.to_owned()),
        (b'M', message),
        (b't', table.to_owned()),
        (b'c', column.to_owned()),
        (b'R', "cipherstash-proxy".to_owned()),
    ])
}

pub fn system_error(message: String) -> DiagnosticResponse {
    standard("FATAL", CODE_SYSTEM_ERROR, message)
}

pub fn is_fatal(response: &DiagnosticResponse) -> bool {
    response
        .fields
        .iter()
        .any(|field| field.code == b'S' && field.value.as_ref() == b"FATAL")
}

fn extract_line_from_parse_error(message: &str) -> Option<usize> {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*Line:\s*(\d+)").unwrap());
    RE.captures(message)
        .and_then(|capture| capture.get(1)?.as_str().parse().ok())
}

fn extract_position_from_parse_error(message: &str) -> Option<usize> {
    static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\s*Column:\s*(\d+)").unwrap());
    RE.captures(message)
        .and_then(|capture| capture.get(1)?.as_str().parse().ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(response: &DiagnosticResponse, code: u8) -> Option<&[u8]> {
        response
            .fields
            .iter()
            .find(|field| field.code == code)
            .map(|field| field.value.as_ref())
    }

    #[test]
    fn sql_parse_error_includes_line_and_position() {
        let response =
            invalid_sql_statement("sql syntax error in blah Line: 1, Column: 2".to_owned());
        assert_eq!(field(&response, b'L'), Some(b"1".as_slice()));
        assert_eq!(field(&response, b'P'), Some(b"2".as_slice()));
    }
}
