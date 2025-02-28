use std::{cell::RefCell, rc::Rc};

use sqlparser::ast::Ident;

use crate::inference::Type;

#[derive(Debug, Clone, Eq, PartialEq)]
pub(crate) struct Relation {
    pub(crate) projection_type: Rc<RefCell<Type>>,
    pub(crate) name: Option<Ident>,
}
