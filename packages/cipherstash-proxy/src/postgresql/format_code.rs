#[derive(Clone, Copy, Debug, PartialEq)]
pub enum FormatCode {
    Text,
    Binary,
}

impl From<i16> for FormatCode {
    fn from(value: i16) -> Self {
        match value {
            1 => FormatCode::Binary,
            _ => FormatCode::Text,
        }
    }
}

impl From<FormatCode> for i16 {
    fn from(value: FormatCode) -> Self {
        value as i16
    }
}
