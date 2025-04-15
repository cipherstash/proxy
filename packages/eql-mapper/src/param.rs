use std::fmt::Display;

#[derive(Debug, Copy, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Param(pub u16);

#[derive(Debug, thiserror::Error, Eq, PartialEq)]
pub enum ParamError {
    #[error("Invalid param format '{}'; expected '$1' for example", _0)]
    InvalidParamFormat(String),
}

impl TryFrom<&String> for Param {
    type Error = ParamError;

    fn try_from(value: &String) -> Result<Self, Self::Error> {
        match value.replace("$", "").parse::<u16>() {
            Ok(n) => Ok(Self(n)),
            Err(_) => Err(ParamError::InvalidParamFormat(value.to_string())),
        }
    }
}

impl Display for Param {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&format!("${}", self.0))
    }
}