//! The correspondence between the params of the input statement and the params
//! of the rewritten one.
//!
//! There is no 1:1 guarantee. An output param may be **derived from more than
//! one input param** — encrypted JSON equality (`col -> $1 = $2`) fuses a path
//! and a value into a single value-selector needle, so two input params become
//! one output param. Nothing in the pipeline may assume that param `$n` on the
//! wire to PostgreSQL is param `$n` as the client wrote it.
//!
//! A [`ParamPlan`] is the explicit statement of that correspondence: one
//! [`OutputParam`] per placeholder in the rewritten SQL, each naming the input
//! params its value is built from. The proxy binds against the plan — it
//! describes the *input* params to the client, and sends the *output* params to
//! PostgreSQL.

use std::collections::HashSet;

use crate::unifier::Value;
use crate::{EqlMapperError, JsonSelectorSource, Param};

/// How the value bound to one output param is derived from the input params.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OutputParamSource {
    /// Forwarded from a single input param, unchanged in meaning (though still
    /// encrypted if the param is EQL-typed).
    Input(Param),

    /// Fused from several operands into one encrypted value-selector needle:
    /// the JSON path and the value it must equal. Each step of the path is
    /// itself a param or a literal in the SQL; the value is always the param
    /// carrying this output.
    ///
    /// See [`crate::JsonValueSelectors`].
    JsonValueSelector {
        path: JsonSelectorSource,
        value: Param,
    },
}

impl OutputParamSource {
    /// Every input param this output param consumes.
    pub fn inputs(&self) -> Vec<Param> {
        match self {
            OutputParamSource::Input(param) => vec![*param],
            OutputParamSource::JsonValueSelector { path, value } => {
                path.params().chain([*value]).collect()
            }
        }
    }
}

/// One placeholder of the rewritten statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputParam {
    /// Its 1-based position in the rewritten SQL.
    pub param: Param,

    /// The type of the value to bind — what the proxy must encrypt it as.
    pub value: Value,

    /// The input params it is built from.
    pub source: OutputParamSource,

    /// Whether this param is a **query operand** — an operand of a predicate,
    /// which must reach PostgreSQL carrying only search terms and no
    /// ciphertext. See [`crate::QueryOperands`].
    pub query_operand: bool,
}

/// The params of the rewritten statement, in the order PostgreSQL will see them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParamPlan {
    outputs: Vec<OutputParam>,
}

impl ParamPlan {
    pub(crate) fn new(outputs: Vec<OutputParam>) -> Self {
        Self { outputs }
    }

    pub fn outputs(&self) -> &[OutputParam] {
        &self.outputs
    }

    pub fn len(&self) -> usize {
        self.outputs.len()
    }

    pub fn is_empty(&self) -> bool {
        self.outputs.is_empty()
    }

    /// `true` when every input param maps to the output param of the same
    /// position — the ordinary case, where the rewrite changed no param.
    pub fn is_identity(&self) -> bool {
        self.outputs.iter().enumerate().all(|(idx, output)| {
            matches!(output.source, OutputParamSource::Input(input)
                if input.0 as usize == idx + 1)
        })
    }

    /// Checks that the plan consumes every input param.
    ///
    /// The invariant the rewrite must uphold is **coverage, not exactly-once**:
    /// an input may feed several output params (if the rewrite duplicated it),
    /// and several inputs may feed one output (fusion), but an input that feeds
    /// nothing has been silently dropped — the client would bind a value that
    /// never reaches PostgreSQL and never contributes to a needle, which is a
    /// bug in a rewrite rule rather than a valid query.
    pub(crate) fn check_covers(&self, inputs: &[Param]) -> Result<(), EqlMapperError> {
        let consumed: HashSet<Param> = self
            .outputs
            .iter()
            .flat_map(|output| output.source.inputs())
            .collect();

        if let Some(orphan) = inputs.iter().find(|param| !consumed.contains(param)) {
            return Err(EqlMapperError::InternalError(format!(
                "param {orphan} of the input statement is not consumed by the rewritten statement"
            )));
        }

        Ok(())
    }
}
