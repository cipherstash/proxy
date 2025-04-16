use sqlparser::ast::Ident;

use crate::TypeVar;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct Relation {
    pub(crate) projection_type: TypeVar,
    pub(crate) name: Option<Ident>,
}
