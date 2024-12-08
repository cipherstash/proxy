pub mod auth;
pub mod bind;
pub mod error_response;
pub mod parse;
pub mod query;

/// Protocol message codes.
// pub const BIND: u8 = b'B';
pub const PARSE: u8 = b'P';
// pub const QUERY: u8 = b'Q';
pub const NULL: i32 = -1;

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FrontendCode {
    Query,
    Parse,
    Bind,
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
            'Q' => FrontendCode::Query,
            'P' => FrontendCode::Parse,
            'B' => FrontendCode::Bind,
            _ => FrontendCode::Unknown(code as char),
        }
    }
}

impl From<FrontendCode> for u8 {
    fn from(code: FrontendCode) -> Self {
        match code {
            FrontendCode::Bind => b'B',
            FrontendCode::Parse => b'P',
            FrontendCode::Query => b'Q',
            FrontendCode::Unknown(c) => c as u8,
        }
    }
}

impl From<FrontendCode> for char {
    fn from(code: FrontendCode) -> Self {
        match code {
            FrontendCode::Bind => 'B',
            FrontendCode::Parse => 'P',
            FrontendCode::Query => 'Q',
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

#[derive(Debug, Clone)]
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
