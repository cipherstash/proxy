use super::Column;
use std::collections::HashMap;

/// Where the path half of a fused JSON value selector comes from, resolved to
/// this statement's bind params.
///
/// The proxy's copy of [`eql_mapper::JsonSelectorSource`], with param numbers
/// converted to 0-based bind indexes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonSelectorPath {
    /// A literal path in the SQL, known at Parse time.
    Literal(String),

    /// A placeholder path, arriving in this (0-based) bind param.
    Param(usize),
}

///
/// Type Analysed parameters and projection
///
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    pub param_columns: Vec<Option<Column>>,
    pub projection_columns: Vec<Option<Column>>,
    pub literal_columns: Vec<Option<Column>>,
    pub postgres_param_types: Vec<i32>,

    /// Params that carry a fused JSON value-selector needle (`col -> sel =
    /// $n`), keyed by 0-based bind index, mapped to where their path comes from.
    ///
    /// Bind consults this to compose `{"path", "value"}` from two params before
    /// encrypting. Empty for every statement without encrypted JSON equality.
    pub json_value_selectors: HashMap<usize, JsonSelectorPath>,
}

impl Statement {
    pub fn new(
        param_columns: Vec<Option<Column>>,
        projection_columns: Vec<Option<Column>>,
        literal_columns: Vec<Option<Column>>,
        postgres_param_types: Vec<i32>,
        json_value_selectors: HashMap<usize, JsonSelectorPath>,
    ) -> Statement {
        Statement {
            param_columns,
            projection_columns,
            literal_columns,
            postgres_param_types,
            json_value_selectors,
        }
    }

    /// The 0-based bind indexes of params that supply only a value-selector
    /// path. The rewrite folds these into the needle and drops them from the
    /// SQL, so PostgreSQL never references them — but it still needs their type
    /// declared in Parse to prepare the statement.
    pub fn unreferenced_param_indexes(&self) -> Vec<usize> {
        self.json_value_selectors
            .values()
            .filter_map(|path| match path {
                JsonSelectorPath::Param(idx) => Some(*idx),
                JsonSelectorPath::Literal(_) => None,
            })
            .collect()
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
