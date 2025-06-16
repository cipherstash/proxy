use std::marker::PhantomData;

use sqltk::parser::ast::{Expr, Value, ValueWithSpan};

use crate::EqlMapperError;

use super::TransformationRule;

/// Rule that fails if an a [`Value::Placeholder`] has *already* been replaced.
///
/// This rule never the AST.
///
/// This is an internal sanity check - it should never happen if there are no bugs in EQL mapping.
// TODO: this rule should be changed to a postcondition check.
#[derive(Debug)]
pub(crate) struct FailOnPlaceholderChange<'ast> {
    _ast: PhantomData<&'ast ()>,
}

impl<'ast> FailOnPlaceholderChange<'ast> {
    pub(crate) fn new() -> Self {
        Self { _ast: PhantomData }
    }

    fn check_no_placeholders_have_been_modified<N: sqltk::Visitable>(
        &self,
        node_path: &sqltk::NodePath<'ast>,
        target_node: &N,
    ) -> Result<(), crate::EqlMapperError> {
        if let Some((expr,)) = node_path.last_1_as::<Expr>() {
            let target_node = target_node.downcast_ref::<Expr>().unwrap();

            if let (
                Expr::Value(
                    source_value @ ValueWithSpan {
                        value: Value::Placeholder(_),
                        ..
                    },
                ),
                Expr::Value(target_value),
            ) = (expr, target_node)
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

impl<'ast> TransformationRule<'ast> for FailOnPlaceholderChange<'ast> {
    fn apply<N: sqltk::Visitable>(
        &mut self,
        node_path: &sqltk::NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, crate::EqlMapperError> {
        self.check_no_placeholders_have_been_modified(node_path, target_node)?;
        Ok(false)
    }

    fn would_edit<N: sqltk::Visitable>(
        &mut self,
        node_path: &sqltk::NodePath<'ast>,
        target_node: &N,
    ) -> bool {
        if self
            .check_no_placeholders_have_been_modified(node_path, target_node)
            .is_err()
        {
            return true;
        }
        false
    }
}
