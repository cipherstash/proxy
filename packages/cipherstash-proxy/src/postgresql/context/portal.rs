use super::{super::format_code::FormatCode, Column, SessionId};
use crate::postgresql::context::statement::Statement;
use std::sync::Arc;

#[derive(Clone, Debug)]
pub enum Portal {
    Encrypted {
        format_codes: Vec<FormatCode>,
        statement: Arc<Statement>,
        session_id: Option<SessionId>,
    },
    Passthrough {
        session_id: Option<SessionId>,
    },
}

impl Portal {
    pub fn encrypted_with_format_codes(
        statement: Arc<Statement>,
        format_codes: Vec<FormatCode>,
        session_id: Option<SessionId>,
    ) -> Portal {
        Portal::Encrypted {
            statement,
            format_codes,
            session_id,
        }
    }

    pub fn encrypted(statement: Arc<Statement>, session_id: Option<SessionId>) -> Portal {
        let format_codes = vec![];
        Portal::Encrypted {
            statement,
            format_codes,
            session_id,
        }
    }

    pub fn passthrough(session_id: Option<SessionId>) -> Portal {
        Portal::Passthrough { session_id }
    }

    pub fn projection_columns(&self) -> &Vec<Option<Column>> {
        static EMPTY: Vec<Option<Column>> = vec![];
        match self {
            Portal::Encrypted { statement, .. } => &statement.projection_columns,
            Portal::Passthrough { .. } => &EMPTY,
        }
    }

    // FormatCodes should not be None at this point
    // FormatCodes will be:
    //  - empty, in which case assume Text
    //  - single value, in which case use this for all columns
    //  - multiple values, in which case use the value for each column
    pub fn format_codes(&self, row_len: usize) -> Vec<FormatCode> {
        match self {
            Portal::Encrypted { format_codes, .. } => match format_codes.len() {
                0 => vec![FormatCode::Text; row_len],
                1 => {
                    let format_code = match format_codes.first() {
                        Some(code) => *code,
                        None => FormatCode::Text,
                    };
                    vec![format_code; row_len]
                }
                _ => format_codes.clone(),
            },
            Portal::Passthrough { .. } => {
                unreachable!()
            }
        }
    }

    pub fn session_id(&self) -> Option<SessionId> {
        match self {
            Portal::Encrypted { session_id, .. } => *session_id,
            Portal::Passthrough { session_id } => *session_id,
        }
    }
}
