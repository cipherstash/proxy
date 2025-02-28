use crate::{
    model::schema::Table,
    pub_types::{Projection, TableColumn},
};

#[derive(Debug, Clone, Eq, PartialEq)]
pub enum Provenance {
    Select(SelectProvenance),
    Insert(InsertProvenance),
    Update(UpdateProvenance),
    Delete(DeleteProvenance),
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct SelectProvenance {
    pub projection: Projection,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct InsertProvenance {
    pub into_table: Table,
    pub returning: Option<Projection>,
    pub columns_written: Vec<TableColumn>,
    pub source_projection: Option<Projection>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct UpdateProvenance {
    pub update_table: Table,
    pub returning: Option<Projection>,
    pub columns_written: Vec<TableColumn>,
}

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct DeleteProvenance {
    pub from_table: Table,
    pub returning: Option<Projection>,
}
