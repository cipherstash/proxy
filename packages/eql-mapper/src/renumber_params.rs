//! Renumbering of the rewritten statement's placeholders.
//!
//! A rewrite rule may drop a placeholder (encrypted JSON equality folds the
//! path operand into the value's needle) or duplicate one. Either way the
//! surviving placeholders no longer read `$1..$n` in order, and PostgreSQL
//! requires a statement's params to be exactly `$1..$m` — an unreferenced `$n`
//! is only accepted when its type is declared, and a gap is a statement whose
//! param count no longer matches what the client bound.
//!
//! So after the rewrite rules have run, this pass walks the new statement in
//! SQL order and renumbers the placeholders `$1..$m`, recording which input
//! param each output position came from. That record is the raw material for a
//! [`crate::ParamPlan`].
//!
//! This runs as a **separate pass, after** the rewrite rules, which is what
//! lets [`crate::FailOnPlaceholderChange`] keep enforcing that no rule replaces
//! a placeholder with a literal during the rewrite itself.

use sqltk::parser::ast;
use sqltk::{NodePath, Transform, Visitable};

use crate::{EqlMapperError, Param};

/// Assigns `$1..$m` to the placeholders of a rewritten statement in SQL order.
#[derive(Debug, Default)]
pub(crate) struct RenumberParams {
    /// The input param each output position was renumbered from, in output
    /// order. Index `i` holds the source of `$(i + 1)`.
    sources: Vec<Param>,
}

impl RenumberParams {
    pub(crate) fn new() -> Self {
        Self::default()
    }

    pub(crate) fn into_sources(self) -> Vec<Param> {
        self.sources
    }
}

impl<'ast> Transform<'ast> for RenumberParams {
    type Error = EqlMapperError;

    fn transform<N: Visitable>(
        &mut self,
        _node_path: &NodePath<'ast>,
        mut target_node: N,
    ) -> Result<N, Self::Error> {
        // Transformation is depth-first and visits siblings in order, so
        // placeholders arrive in the order they appear in the rendered SQL.
        if let Some(ast::Value::Placeholder(name)) = target_node.downcast_mut::<ast::Value>() {
            let source = Param::try_from(&*name)?;
            self.sources.push(source);
            *name = format!("${}", self.sources.len());
        }

        Ok(target_node)
    }
}
