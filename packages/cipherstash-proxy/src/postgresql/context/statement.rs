use super::Column;

///
/// Type Analysed parameters and projection
///
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub param_columns: Vec<Option<Column>>,
    pub projection_columns: Vec<Option<Column>>,
    pub literal_columns: Vec<Option<Column>>,
    pub postgres_param_types: Vec<i32>,
}

impl Statement {
    pub fn new(
        param_columns: Vec<Option<Column>>,
        projection_columns: Vec<Option<Column>>,
        literal_columns: Vec<Option<Column>>,
        postgres_param_types: Vec<i32>,
    ) -> Statement {
        Statement {
            param_columns,
            projection_columns,
            literal_columns,
            postgres_param_types,
        }
    }

    pub fn has_literals(&self) -> bool {
        !self.literal_columns.is_empty()
    }

    pub fn has_params(&self) -> bool {
        !self.param_columns.is_empty()
    }

    pub fn has_projection(&self) -> bool {
        !self.projection_columns.is_empty()
    }
}
