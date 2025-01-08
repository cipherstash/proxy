use sqlparser::ast::Ident;

use crate::unifier::TypeCell;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct Relation {
    pub(crate) projection_type: TypeCell,
    pub(crate) name: Option<Ident>,
}
