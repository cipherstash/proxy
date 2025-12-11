use std::collections::HashMap;
use std::sync::Arc;

use sqltk::parser::ast::{
    BinaryOperator, Expr, Function, FunctionArg, FunctionArgExpr, FunctionArgumentList,
    FunctionArguments, Ident, ObjectName, ObjectNamePart,
};
use sqltk::{AsNodeKey, NodeKey, NodePath, Visitable};

use crate::unifier::{Type, Value};
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
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> RewriteContainmentOps<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }

    /// Returns true if either operand is an EQL type.
    fn uses_eql_type(&self, left: &Expr, right: &Expr) -> bool {
        matches!(
            self.node_types.get(&left.as_node_key()),
            Some(Type::Value(Value::Eql(_)))
        ) || matches!(
            self.node_types.get(&right.as_node_key()),
            Some(Type::Value(Value::Eql(_)))
        )
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

                // Clone the boxed expressions (Expr doesn't implement Default,
                // so we can't use mem::take on Box<Expr>)
                let left_expr = left.as_ref().clone();
                let right_expr = right.as_ref().clone();
                *expr = Self::make_function_call(fn_name, left_expr, right_expr);
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, _node_path: &NodePath<'ast>, target_node: &N) -> bool {
        if let Some(expr) = target_node.downcast_ref::<Expr>() {
            if let Expr::BinaryOp { left, op, right } = expr {
                if matches!(op, BinaryOperator::AtArrow | BinaryOperator::ArrowAt) {
                    return self.uses_eql_type(left, right);
                }
            }
        }
        false
    }
}
