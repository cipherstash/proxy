use std::fmt;

use bytes::BytesMut;

pub mod authentication;
pub mod bind;
pub mod close;
pub mod data_row;
pub mod describe;
pub mod error_response;
pub mod execute;
pub mod name;
pub mod param_description;
pub mod parse;
pub mod query;
pub mod ready_for_query;
pub mod row_description;
pub mod target;
pub mod terminate;

// Re-export commonly used types
pub use name::Name;
pub use target::Target;

pub const NULL: i32 = -1;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FrontendCode {
    Bind,
    Close,
    Describe,
    Execute,
    Flush,
    Parse,
    PasswordMessage,
    Query,
    SASLInitialResponse,
    SASLResponse,
    Sync,
    Terminate,
    Unknown(char),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BackendCode {
    Authentication,
    BindComplete,
    BackendKeyData,
    CloseComplete,
    CommandComplete,
    CopyBothResponse,
    CopyInResponse,
    CopyOutResponse,
    DataRow,
    EmptyQueryResponse,
    ErrorResponse,
    NoData,
    NoticeResponse,
    NotificationResponse,
    ParameterDescription,
    ParameterStatus,
    ParseComplete,
    PortalSuspended,
    ReadyForQuery,
    RowDescription,
    Unknown(char),
}

impl From<u8> for FrontendCode {
    fn from(code: u8) -> Self {
        (code as char).into()
    }
}

impl From<char> for FrontendCode {
    fn from(code: char) -> Self {
        match code {
            'B' => FrontendCode::Bind,
            'C' => FrontendCode::Close,
            'D' => FrontendCode::Describe,
            'E' => FrontendCode::Execute,
            'H' => FrontendCode::Flush,
            'p' => FrontendCode::PasswordMessage,
            'P' => FrontendCode::Parse,
            'Q' => FrontendCode::Query,
            #[allow(unreachable_patterns)]
            'p' => FrontendCode::SASLInitialResponse, // Uses same char, here for completeness
            #[allow(unreachable_patterns)]
            'p' => FrontendCode::SASLResponse, // Uses same char, here for completeness
            'S' => FrontendCode::Sync,
            'X' => FrontendCode::Terminate,
            _ => FrontendCode::Unknown(code),
        }
    }
}

impl From<FrontendCode> for u8 {
    fn from(code: FrontendCode) -> Self {
        match code {
            FrontendCode::Bind => b'B',
            FrontendCode::Close => b'C',
            FrontendCode::Describe => b'D',
            FrontendCode::Execute => b'E',
            FrontendCode::Flush => b'F',
            FrontendCode::Parse => b'P',
            FrontendCode::PasswordMessage => b'p',
            FrontendCode::Query => b'Q',
            FrontendCode::SASLInitialResponse => b'p',
            FrontendCode::SASLResponse => b'p',
            FrontendCode::Sync => b'S',
            FrontendCode::Terminate => b'X',
            FrontendCode::Unknown(c) => c as u8,
        }
    }
}

impl From<FrontendCode> for char {
    fn from(code: FrontendCode) -> Self {
        match code {
            FrontendCode::Bind => 'B',
            FrontendCode::Close => 'C',
            FrontendCode::Describe => 'D',
            FrontendCode::Execute => 'E',
            FrontendCode::Flush => 'F',
            FrontendCode::Parse => 'P',
            FrontendCode::PasswordMessage => 'p',
            FrontendCode::Query => 'Q',
            FrontendCode::SASLInitialResponse => 'p',
            FrontendCode::SASLResponse => 'p',
            FrontendCode::Sync => 'S',
            FrontendCode::Terminate => 'X',
            FrontendCode::Unknown(c) => c,
        }
    }
}

impl From<u8> for BackendCode {
    fn from(code: u8) -> Self {
        match code as char {
            'R' => BackendCode::Authentication,
            'K' => BackendCode::BackendKeyData,
            '2' => BackendCode::BindComplete,
            '3' => BackendCode::CloseComplete,
            'C' => BackendCode::CommandComplete,
            'W' => BackendCode::CopyBothResponse,
            'G' => BackendCode::CopyInResponse,
            'H' => BackendCode::CopyOutResponse,
            'D' => BackendCode::DataRow,
            'I' => BackendCode::EmptyQueryResponse,
            'E' => BackendCode::ErrorResponse,
            'n' => BackendCode::NoData,
            'N' => BackendCode::NoticeResponse,
            'A' => BackendCode::NotificationResponse,
            't' => BackendCode::ParameterDescription,
            'S' => BackendCode::ParameterStatus,
            '1' => BackendCode::ParseComplete,
            's' => BackendCode::PortalSuspended,
            'Z' => BackendCode::ReadyForQuery,
            'T' => BackendCode::RowDescription,
            _ => BackendCode::Unknown(code as char),
        }
    }
}

impl From<BackendCode> for u8 {
    fn from(code: BackendCode) -> Self {
        match code {
            BackendCode::Authentication => b'R',
            BackendCode::BackendKeyData => b'K',
            BackendCode::BindComplete => b'2',
            BackendCode::CloseComplete => b'3',
            BackendCode::CommandComplete => b'C',
            BackendCode::CopyBothResponse => b'W',
            BackendCode::CopyInResponse => b'G',
            BackendCode::CopyOutResponse => b'H',
            BackendCode::DataRow => b'D',
            BackendCode::EmptyQueryResponse => b'I',
            BackendCode::ErrorResponse => b'E',
            BackendCode::NoData => b'n',
            BackendCode::NoticeResponse => b'N',
            BackendCode::NotificationResponse => b'A',
            BackendCode::ParameterDescription => b't',
            BackendCode::ParameterStatus => b'S',
            BackendCode::ParseComplete => b'1',
            BackendCode::PortalSuspended => b's',
            BackendCode::ReadyForQuery => b'Z',
            BackendCode::RowDescription => b'T',
            BackendCode::Unknown(c) => c as u8,
        }
    }
}

impl From<BackendCode> for char {
    fn from(code: BackendCode) -> Self {
        match code {
            BackendCode::Authentication => 'R',
            BackendCode::BackendKeyData => 'K',
            BackendCode::BindComplete => '2',
            BackendCode::CloseComplete => '3',
            BackendCode::CommandComplete => 'C',
            BackendCode::CopyBothResponse => 'W',
            BackendCode::CopyInResponse => 'G',
            BackendCode::CopyOutResponse => 'H',
            BackendCode::DataRow => 'D',
            BackendCode::EmptyQueryResponse => 'I',
            BackendCode::ErrorResponse => 'E',
            BackendCode::NoData => 'n',
            BackendCode::NoticeResponse => 'N',
            BackendCode::NotificationResponse => 'A',
            BackendCode::ParameterDescription => 't',
            BackendCode::ParameterStatus => 'S',
            BackendCode::ParseComplete => '1',
            BackendCode::PortalSuspended => 's',
            BackendCode::ReadyForQuery => 'Z',
            BackendCode::RowDescription => 'T',
            BackendCode::Unknown(c) => c,
        }
    }
}

impl fmt::Display for BackendCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackendCode::Authentication => write!(f, "BackendCode::Authentication"),
            BackendCode::BackendKeyData => write!(f, "BackendCode::BackendKeyData"),
            BackendCode::BindComplete => write!(f, "BackendCode::BindComplete"),
            BackendCode::CloseComplete => write!(f, "BackendCode::CloseComplete"),
            BackendCode::CommandComplete => write!(f, "BackendCode::CommandComplete"),
            BackendCode::CopyBothResponse => write!(f, "BackendCode::CopyBothResponse"),
            BackendCode::CopyInResponse => write!(f, "BackendCode::CopyInResponse"),
            BackendCode::CopyOutResponse => write!(f, "BackendCode::CopyOutResponse"),
            BackendCode::DataRow => write!(f, "BackendCode::DataRow"),
            BackendCode::EmptyQueryResponse => write!(f, "BackendCode::EmptyQueryResponse"),
            BackendCode::ErrorResponse => write!(f, "BackendCode::ErrorResponse"),
            BackendCode::NoData => write!(f, "BackendCode::NoData"),
            BackendCode::NoticeResponse => write!(f, "BackendCode::NoticeResponse"),
            BackendCode::NotificationResponse => write!(f, "BackendCode::NotificationResponse"),
            BackendCode::ParameterDescription => write!(f, "BackendCode::ParameterDescription"),
            BackendCode::ParameterStatus => write!(f, "BackendCode::ParameterStatus"),
            BackendCode::ParseComplete => write!(f, "BackendCode::ParseComplete"),
            BackendCode::PortalSuspended => write!(f, "BackendCode::PortalSuspended"),
            BackendCode::ReadyForQuery => write!(f, "BackendCode::ReadyForQuery"),
            BackendCode::RowDescription => write!(f, "BackendCode::RowDescription"),
            BackendCode::Unknown(c) => write!(f, "BackendCode::Unknown('{}')", c),
        }
    }
}

impl fmt::Display for FrontendCode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FrontendCode::Bind => write!(f, "FrontendCode::Bind"),
            FrontendCode::Close => write!(f, "FrontendCode::Close"),
            FrontendCode::Describe => write!(f, "FrontendCode::Describe"),
            FrontendCode::Execute => write!(f, "FrontendCode::Execute"),
            FrontendCode::Flush => write!(f, "FrontendCode::Flush"),
            FrontendCode::Parse => write!(f, "FrontendCode::Parse"),
            FrontendCode::PasswordMessage => write!(f, "FrontendCode::PasswordMessage"),
            FrontendCode::Query => write!(f, "FrontendCode::Query"),
            FrontendCode::SASLInitialResponse => write!(f, "FrontendCode::SASLInitialResponse"),
            FrontendCode::SASLResponse => write!(f, "FrontendCode::SASLResponse"),
            FrontendCode::Sync => write!(f, "FrontendCode::Sync"),
            FrontendCode::Terminate => write!(f, "FrontendCode::Terminate"),
            FrontendCode::Unknown(c) => write!(f, "FrontendCode::Unknown('{}')", c),
        }
    }
}

///
/// Peaks at the first byte char.
/// Assumes that a leading `{` may be a JSON value
/// The Plaintext Payload is always a JSON object so this is a pretty naive approach
/// We are not worried about an exhaustive check here
///
pub fn maybe_json(bytes: &BytesMut) -> bool {
    if bytes.is_empty() {
        return false;
    }

    let b = bytes.as_ref()[0];
    b == b'{'
}

///
/// Postgres binary json is regular json with a leading header byte
/// The header byte is always 1
///
pub fn maybe_jsonb(bytes: &BytesMut) -> bool {
    // Empty JSONB is at least 3 bytes
    // `1{}``
    if bytes.len() <= 3 {
        return false;
    }

    let b = bytes.as_ref();

    let header = b[0];
    let first = b[1];
    header == 1 && first == b'{'
}
