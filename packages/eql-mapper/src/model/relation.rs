use std::{cell::RefCell, rc::Rc};

use sqlparser::ast::Ident;

use crate::inference::Type;

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Relation {
    pub projection_type: Rc<RefCell<Type>>,
    pub name: Option<Ident>,
}
