use std::{cell::RefCell, rc::Rc, sync::Arc};

use crate::{inference::ProjectionColumn, model::schema::Table};

use super::TableColumn;

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Provenance {
    Select(SelectProvenance),
    Insert(InsertProvenance),
    Update(UpdateProvenance),
    Delete(DeleteProvenance),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SelectProvenance {
    pub projection: Rc<RefCell<Vec<ProjectionColumn>>>,
    pub projection_table_columns: Vec<TableColumn>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct InsertProvenance {
    pub into_table: Arc<Table>,
    pub returning: Option<Rc<RefCell<Vec<ProjectionColumn>>>>,
    pub returning_table_columns: Option<Vec<TableColumn>>,
    pub columns_written: Vec<TableColumn>,
    pub source_projection: Option<Rc<RefCell<Vec<ProjectionColumn>>>>,
    pub source_table_columns: Option<Vec<TableColumn>>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UpdateProvenance {
    pub update_table: Arc<Table>,
    pub returning: Option<Rc<RefCell<Vec<ProjectionColumn>>>>,
    pub returning_table_columns: Option<Vec<TableColumn>>,
    pub columns_written: Vec<TableColumn>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DeleteProvenance {
    pub from_table: Arc<Table>,
    pub returning: Option<Rc<RefCell<Vec<ProjectionColumn>>>>,
    pub returning_table_columns: Option<Vec<TableColumn>>,
}
