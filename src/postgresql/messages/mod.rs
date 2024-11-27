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
