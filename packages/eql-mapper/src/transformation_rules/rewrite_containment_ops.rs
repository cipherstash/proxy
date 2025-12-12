use std::collections::HashMap;
use std::mem;
use std::sync::Arc;

use sqltk::parser::ast::{
    BinaryOperator, Expr, Function, FunctionArg, FunctionArgExpr, FunctionArgumentList,
    FunctionArguments, Ident, ObjectName, ObjectNamePart, Value, ValueWithSpan,
};
use sqltk::parser::tokenizer::Span;
use sqltk::{NodeKey, NodePath, Visitable};

use crate::unifier::Type;
use crate::EqlMapperError;

use super::TransformationRule;

/// Rewrites `@>` and `<@` operators on EQL types to function calls.
///
/// - `col @> val` → `eql_v2.jsonb_contains(col, val)`
/// - `val <@ col` → `eql_v2.jsonb_contained_by(val, col)`
///
/// This transformation enables GIN index usage when the index is created on
/// `eql_v2.jsonb_array(encrypted_col)`.
#[derive(Debug)]
pub struct RewriteContainmentOps<'ast> {
    #[allow(dead_code)]
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> RewriteContainmentOps<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }

    fn make_function_call(fn_name: &str, left: Expr, right: Expr) -> Expr {
        Expr::Function(Function {
            name: ObjectName(vec![
                ObjectNamePart::Identifier(Ident::new("eql_v2")),
                ObjectNamePart::Identifier(Ident::new(fn_name)),
            ]),
            uses_odbc_syntax: false,
            args: FunctionArguments::List(FunctionArgumentList {
                args: vec![
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(left)),
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(right)),
                ],
                duplicate_treatment: None,
                clauses: vec![],
            }),
            parameters: FunctionArguments::None,
            filter: None,
            null_treatment: None,
            over: None,
            within_group: vec![],
        })
    }
}

impl<'ast> TransformationRule<'ast> for RewriteContainmentOps<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if self.would_edit(node_path, target_node) {
            let expr = target_node.downcast_mut::<Expr>().unwrap();
            if let Expr::BinaryOp { left, op, right } = expr {
                let fn_name = match op {
                    BinaryOperator::AtArrow => "jsonb_contains",     // @>
                    BinaryOperator::ArrowAt => "jsonb_contained_by", // <@
                    _ => return Ok(false),
                };

                // Use mem::replace to move (not copy) the original nodes,
                // preserving their NodeKey identity for downstream casting rules
                let dummy = Expr::Value(ValueWithSpan {
                    value: Value::Null,
                    span: Span::empty(),
                });
                let left_expr = mem::replace(&mut **left, dummy.clone());
                let right_expr = mem::replace(&mut **right, dummy);
                *expr = Self::make_function_call(fn_name, left_expr, right_expr);
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, _node_path: &NodePath<'ast>, target_node: &N) -> bool {
        if let Some(expr) = target_node.downcast_ref::<Expr>() {
            if let Expr::BinaryOp { left: _, op, right: _ } = expr {
                if matches!(op, BinaryOperator::AtArrow | BinaryOperator::ArrowAt) {
                    // Always rewrite containment operators - if they're used in the query,
                    // at least one operand must be EQL-typed based on schema
                    return true;
                }
            }
        }
        false
    }
}
