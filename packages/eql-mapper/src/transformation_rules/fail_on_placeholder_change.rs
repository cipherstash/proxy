use sqlparser::ast::{Expr, Value};

use crate::EqlMapperError;

use super::{
    TransformationRule,
};

/// Rule that fails if an a [`Value::Placeholder`] has been replaced.
///
/// This is an internal sanity check - it should never happen if there are no bugs in EQL mapping.
#[derive(Debug)]
pub(crate) struct FailOnPlaceholderChange;

impl<'ast> TransformationRule<'ast> for FailOnPlaceholderChange {
    fn apply<N: sqltk::Visitable>(
        &mut self,
        node_path: &sqltk::NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<(), crate::EqlMapperError> {
        if let Some((expr,)) = node_path.last_1_as::<Expr>() {
            let target_node = target_node.downcast_mut::<Expr>().unwrap();

            if let (
                Expr::Value(source_value @ Value::Placeholder(_)),
                Expr::Value(target_value),
            ) = (expr, &*target_node)
            {
                if source_value != target_value {
                    return Err(EqlMapperError::InternalError(
                        "attempt was made to update placeholder with literal".to_string(),
                    ));
                }
            }
        }

        Ok(())
    }
}
