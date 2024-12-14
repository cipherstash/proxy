use sqlparser::ast::Ident;

use crate::inference::unifier::Type;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct Relation {
    pub(crate) projection_type: Type,
    pub(crate) name: Option<Ident>,
}
