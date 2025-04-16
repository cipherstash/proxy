use std::sync::Arc;

use sqlparser::ast::Ident;

use crate::unifier::Type;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct Relation {
    pub(crate) projection_type: Arc<Type>,
    pub(crate) name: Option<Ident>,
}
