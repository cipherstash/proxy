use sqlparser::ast::Ident;

use crate::TID;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct Relation {
    pub(crate) projection_type: TID,
    pub(crate) name: Option<Ident>,
}
