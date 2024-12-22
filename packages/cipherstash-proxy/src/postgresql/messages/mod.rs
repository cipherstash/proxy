use bytes::BytesMut;

pub mod authentication;
pub mod bind;
pub mod data_row;
pub mod describe;
pub mod error_response;
pub mod param_description;
pub mod parse;
pub mod query;
pub mod row_description;

pub const NULL: i32 = -1;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FrontendCode {
    Bind,
    Describe,
    Parse,
    PasswordMessage,
    Query,
    SASLInitialResponse,
    SASLResponse,
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
            'D' => FrontendCode::Describe,
            'p' => FrontendCode::PasswordMessage,
            'P' => FrontendCode::Parse,
            'Q' => FrontendCode::Query,
            #[allow(unreachable_patterns)]
            'p' => FrontendCode::SASLInitialResponse, // Uses same char, here for completeness
            #[allow(unreachable_patterns)]
            'p' => FrontendCode::SASLResponse, // Uses same char, here for completeness
            // 'p' => FrontendCode::SASLResponse,
            _ => FrontendCode::Unknown(code),
        }
    }
}

impl From<FrontendCode> for u8 {
    fn from(code: FrontendCode) -> Self {
        match code {
            FrontendCode::Bind => b'B',
            FrontendCode::Describe => b'D',
            FrontendCode::Parse => b'P',
            FrontendCode::PasswordMessage => b'p',
            FrontendCode::Query => b'Q',
            FrontendCode::SASLInitialResponse => b'p',
            FrontendCode::SASLResponse => b'p',
            FrontendCode::Unknown(c) => c as u8,
        }
    }
}

impl From<FrontendCode> for char {
    fn from(code: FrontendCode) -> Self {
        match code {
            FrontendCode::Bind => 'B',
            FrontendCode::Describe => 'D',
            FrontendCode::Parse => 'P',
            FrontendCode::PasswordMessage => 'p',
            FrontendCode::Query => 'Q',
            FrontendCode::SASLInitialResponse => 'p',
            FrontendCode::SASLResponse => 'p',
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

#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub enum Destination {
    Named(String),
    Unnamed,
}

impl Destination {
    pub fn new(name: String) -> Destination {
        if name.is_empty() {
            Destination::Unnamed
        } else {
            Destination::Named(name)
        }
    }

    pub fn as_str(&self) -> &str {
        match self {
            Destination::Named(name) => name,
            Destination::Unnamed => "",
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
    let b = bytes.as_ref()[0];
    b == b'{'
}

///
/// Postgres binary json is regular json with a leading header byte
/// The header byte is always 1
///
pub fn maybe_jsonb(bytes: &BytesMut) -> bool {
    let header = bytes.as_ref()[0];
    let first = bytes.as_ref()[1];
    header == 1 && first == b'{'
}
