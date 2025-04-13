use sqlparser::ast::{Expr, Value};

use crate::EqlMapperError;

use super::{
    selector::{MatchTarget, Selector},
    Rule,
};

/// Rule that fails if an a [`Value::Placeholder`] has been replaced.
///
/// This is an internal sanity check - it should never happen if there are no bugs in EQL mapping.
#[derive(Debug)]
pub(crate) struct FailOnPlaceholderChange;

impl<'ast> Rule<'ast> for FailOnPlaceholderChange {
    fn apply<N: sqltk::Visitable>(
        &mut self,
        ctx: &sqltk::Context<'ast>,
        source_node: &'ast N,
        target_node: N,
    ) -> Result<N, crate::EqlMapperError> {
        MatchTarget::<Expr>::on_match_then(
            ctx,
            source_node,
            target_node,
            &mut |source_expr, target_expr| {
                if let (
                    Expr::Value(source_value @ Value::Placeholder(_)),
                    Expr::Value(target_value),
                ) = (source_expr, &target_expr)
                {
                    if source_value != target_value {
                        return Err(EqlMapperError::InternalError(
                            "attempt was made to update placeholder with literal".to_string(),
                        ));
                    }
                }

                Ok(target_expr)
            },
        )
    }
}
