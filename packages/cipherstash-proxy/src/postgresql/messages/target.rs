use crate::error::{Error, ProtocolError};
use std::convert::TryFrom;

///
/// The target of describe or close messages.
///
/// Valid values are PreparedStatement or Portal
///
/// A Portal is a parsed statement PLUS any bound parameters
/// Describe with `Target::Portal` returns the RowDescription describing the result set.
/// The assumption is that the parameters are already bound to the portal, so the Describe message is not required to include any parameter information.
///
/// Calls to Execute are made on a Portal (not a prepared statement) as execute requires any bound parameters
///
/// A Statement is the parsed statement
/// Describe with `Target::Statement` returns a ParameterDescription followed by the RowDescription.
///
///
/// See https://www.postgresql.org/docs/current/protocol-flow.html#PROTOCOL-FLOW-EXT-QUERY
///
#[derive(Debug, Clone)]
pub enum Target {
    Portal,
    Statement,
}

impl TryFrom<u8> for Target {
    type Error = Error;

    fn try_from(t: u8) -> Result<Target, Error> {
        match t as char {
            'S' => Ok(Target::Statement),
            'P' => Ok(Target::Portal),
            t => Err(ProtocolError::UnexpectedDescribeTarget(t).into()),
        }
    }
}

impl From<Target> for u8 {
    fn from(target: Target) -> u8 {
        match target {
            Target::Statement => b'S',
            Target::Portal => b'P',
        }
    }
}
