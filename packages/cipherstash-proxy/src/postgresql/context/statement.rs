use super::Column;
use eql_mapper::{JsonSelectorSegment, ParamPlan};

/// Where one step of the path half of a fused JSON value selector comes from.
///
/// The proxy's copy of [`eql_mapper::JsonSelectorSegment`], with param numbers
/// converted to 0-based bind indexes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JsonSelectorStep {
    /// A literal selector in the SQL, known at Parse time.
    Literal(String),

    /// A placeholder selector, arriving in this (0-based) input param.
    Param(usize),
}

/// The path half of a fused JSON value selector: the steps of the accessor
/// chain, outermost last.
///
/// A chain (`col -> 'a' -> 'b'`) is one path into one document — the payload
/// between the steps is encrypted, so there is nothing for the database to
/// traverse. The steps are resolved and composed into a single eJSONPath at
/// encryption time, once any placeholder step is bound.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JsonSelectorPath {
    pub steps: Vec<JsonSelectorStep>,
}

/// How the value bound to one output param is built from the input params.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputParamSource {
    /// Taken from this (0-based) input param.
    Input(usize),

    /// Fused from a JSON path and a value into one value-selector needle.
    JsonValueSelector {
        path: JsonSelectorPath,
        value: usize,
    },
}

impl OutputParamSource {
    /// The input param this output is built *around* — the one whose wire
    /// format and (for a passthrough) whose bytes it inherits. For a fusion
    /// that is the value operand; the path only contributes to the needle.
    pub fn primary_input(&self) -> usize {
        match self {
            OutputParamSource::Input(idx) => *idx,
            OutputParamSource::JsonValueSelector { value, .. } => *value,
        }
    }
}

/// One param of the *rewritten* statement — what PostgreSQL will be sent.
#[derive(Debug, Clone, PartialEq)]
pub struct OutputParam {
    /// The column configuration, when this param must be encrypted. `None` for
    /// a native param, which is forwarded byte-for-byte.
    pub column: Option<Column>,

    /// The input param(s) its value is built from.
    pub source: OutputParamSource,

    /// Whether this param is a query operand, whose payload must be projected
    /// to carry search terms without a ciphertext.
    pub query_operand: bool,
}

///
/// Type Analysed parameters and projection
///
/// Params have **two** shapes, and they are not guaranteed to correspond:
/// `param_columns` describes what the *client* binds, `output_params` describes
/// what PostgreSQL receives. Encrypted JSON equality fuses two input params into
/// one output param, so any code that assumes `$n` in equals `$n` out is wrong.
///
#[derive(Debug, Clone, PartialEq)]
pub struct Statement {
    /// The params as the client sees them — used to decode bound values and to
    /// answer `Describe`.
    pub param_columns: Vec<Option<Column>>,

    /// The params of the rewritten statement, in the order PostgreSQL sees
    /// them, each naming the input it is built from.
    pub output_params: Vec<OutputParam>,

    pub projection_columns: Vec<Option<Column>>,
    pub literal_columns: Vec<Option<Column>>,
    pub postgres_param_types: Vec<i32>,
}

impl Statement {
    pub fn new(
        param_columns: Vec<Option<Column>>,
        output_params: Vec<OutputParam>,
        projection_columns: Vec<Option<Column>>,
        literal_columns: Vec<Option<Column>>,
        postgres_param_types: Vec<i32>,
    ) -> Statement {
        Statement {
            param_columns,
            output_params,
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

/// `true` when `output_params` are the first `output_params.len()` input params
/// in order, unchanged — the ordinary case, where the rewrite reshaped nothing.
///
/// Callers that hold the bound values must also check the count matches: a
/// prefix match alone would silently drop trailing params.
pub fn params_are_positional(output_params: &[OutputParam]) -> bool {
    output_params
        .iter()
        .enumerate()
        .all(|(idx, output)| output.source == OutputParamSource::Input(idx))
}

/// Converts a mapper [`ParamPlan`] to the proxy's 0-based form, pairing each
/// output param with the column configuration that says how to encrypt it.
///
/// `output_columns` is positional over the plan's outputs.
pub fn output_params_from_plan(
    plan: &ParamPlan,
    output_columns: Vec<Option<Column>>,
) -> Vec<OutputParam> {
    plan.outputs()
        .iter()
        .zip(output_columns)
        .map(|(output, column)| OutputParam {
            column,
            query_operand: output.query_operand,
            source: match &output.source {
                eql_mapper::OutputParamSource::Input(param) => {
                    OutputParamSource::Input(to_index(param.0))
                }
                eql_mapper::OutputParamSource::JsonValueSelector { path, value } => {
                    OutputParamSource::JsonValueSelector {
                        path: JsonSelectorPath {
                            steps: path
                                .segments()
                                .iter()
                                .map(|segment| match segment {
                                    JsonSelectorSegment::Literal(selector) => {
                                        JsonSelectorStep::Literal(selector.to_owned())
                                    }
                                    JsonSelectorSegment::Param(param) => {
                                        JsonSelectorStep::Param(to_index(param.0))
                                    }
                                })
                                .collect(),
                        },
                        value: to_index(value.0),
                    }
                }
            },
        })
        .collect()
}

/// Mapper params are 1-based (`$1`), bind params are 0-based.
fn to_index(param: u16) -> usize {
    param.saturating_sub(1) as usize
}
