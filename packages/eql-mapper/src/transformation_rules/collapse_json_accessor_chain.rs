use std::collections::HashMap;
use std::mem;
use std::sync::Arc;

use sqltk::parser::ast::Value as SqltkValue;
use sqltk::parser::ast::{
    BinaryOperator, Expr, Function, FunctionArg, FunctionArgExpr, FunctionArgumentList,
    FunctionArguments, Ident, ObjectName, ObjectNamePart, ValueWithSpan,
};
use sqltk::parser::tokenizer::Span;
use sqltk::{NodeKey, NodePath, Visitable};

use crate::json_value_selector::json_accessor_chain;
use crate::unifier::{EqlTerm, Type, Value};
use crate::EqlMapperError;

use super::helpers::{cast_encrypted_operand, full_payload_domain};
use super::TransformationRule;

/// Collapses a multi-step encrypted JSON accessor chain into a SINGLE accessor
/// on the root document:
///
/// - `col -> 'a' -> 'b'` → `eql_v3."->"(col, <sel>)`
/// - `col -> 'a' ->> 'b'` → `eql_v3."->>"(col, <sel>)`
/// - `jsonb_path_query_first(col, '$.a') -> 'b'` → `eql_v3."->"(col, <sel>)`
///
/// where `<sel>` is the chain's OUTERMOST selector operand, encrypted to key the
/// whole composed path (`$.a.b`) rather than the one segment its text spells.
/// The proxy composes that path from [`crate::JsonAccessorPaths`], which the type
/// inferencer recorded against this same operand.
///
/// A chain cannot be two hops. `eql_v3."->"` searches the document's `sv` array,
/// and what it returns is one entry with no `sv` of its own — so an accessor
/// applied to the result of an accessor finds nothing and returns NULL. That is
/// the failure this rule exists to prevent, and it is silent: the query runs.
///
/// Only the OUTERMOST node of a chain is rewritten, and it is rewritten in one
/// step from the chain's root, so the intermediate accessors are discarded whole.
/// Every plaintext selector in them goes with them — leaving one behind would ship
/// a field name to PostgreSQL in the clear (CIP-3682) as well as applying native
/// jsonb `->` to an encrypted payload.
///
/// # Relationship to the other JSON rules
///
/// This runs BEFORE [`super::RewriteContainmentOps`], which functionalises a
/// single `->`, and replaces the node with the finished call so that rule declines
/// (it requires its target to still be a `BinaryOp`). Doing it here rather than
/// leaving a one-step `BinaryOp` behind keeps the cast decision keyed to the
/// operand that actually survives: `RewriteContainmentOps` would read the type of
/// the ORIGINAL left operand, which for a chain is the discarded inner accessor.
///
/// For an EQUALITY over a chain this rule still fires, on the accessor below the
/// comparison, and its result is then discarded by
/// [`super::RewriteJsonValueSelectorEq`] — which re-roots the containment at the
/// bare column read from the original AST. Equality keys path and value into ONE
/// needle, which is strictly stronger than an accessor plus a comparison, so it
/// must keep winning.
#[derive(Debug)]
pub struct CollapseJsonAccessorChain<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> CollapseJsonAccessorChain<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }

    /// The root document of the multi-step ENCRYPTED chain at `expr`, or `None`
    /// if this is not one.
    ///
    /// Gated on the node's own type being [`EqlTerm::JsonExtracted`]: that is what
    /// inference assigns to a chain it resolved against an encrypted document, so
    /// it is exactly the set of chains whose path it also recorded. A native
    /// `jsonb` chain is typed `Native` and is left alone — plaintext `jsonb`
    /// genuinely chains, hop by hop.
    fn multi_step_chain(&self, expr: &'ast Expr) -> Option<&'ast Expr> {
        if !matches!(
            self.node_types.get(&NodeKey::new(expr)),
            Some(Type::Value(Value::Eql(EqlTerm::JsonExtracted(_))))
        ) {
            return None;
        }

        json_accessor_chain(expr)
            .filter(|(_, selectors)| selectors.len() > 1)
            .map(|(root, _)| root)
    }

    /// The `eql_v3` function a field access is spelled as. `->` yields the entry,
    /// `->>` its text; the outermost step of the chain decides which, exactly as
    /// it would for a single access.
    fn accessor_fn(op: &BinaryOperator) -> Option<&'static str> {
        match op {
            BinaryOperator::Arrow => Some("->"),
            BinaryOperator::LongArrow => Some("->>"),
            _ => None,
        }
    }

    /// Builds `eql_v3."->"(container, selector)`.
    fn accessor_call(fn_name: &str, container: Expr, selector: Expr) -> Expr {
        Expr::Function(Function {
            name: ObjectName(vec![
                ObjectNamePart::Identifier(Ident::new("eql_v3")),
                ObjectNamePart::Identifier(Ident::with_quote('"', fn_name)),
            ]),
            uses_odbc_syntax: false,
            args: FunctionArguments::List(FunctionArgumentList {
                args: vec![
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(Box::new(container))),
                    FunctionArg::Unnamed(FunctionArgExpr::Expr(Box::new(selector))),
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

impl<'ast> TransformationRule<'ast> for CollapseJsonAccessorChain<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        // Match against the ORIGINAL nodes: `node_types` is keyed by them, and
        // the chain has to be walked before any rule reshapes it.
        let Some((original @ Expr::BinaryOp { op, right, .. },)) = node_path.last_1_as::<Expr>()
        else {
            return Ok(false);
        };

        let Some(fn_name) = Self::accessor_fn(op) else {
            return Ok(false);
        };

        let Some(root) = self.multi_step_chain(original) else {
            return Ok(false);
        };

        let Some(expr) = target_node.downcast_mut::<Expr>() else {
            return Ok(false);
        };
        let Expr::BinaryOp {
            right: target_right,
            ..
        } = expr
        else {
            return Ok(false);
        };

        // The selector is a query operand of the accessor call. `->` takes it as
        // bare encrypted text, so `full_payload_domain` returns `None` for it and
        // no cast is applied — the call is here so the choice stays with the rule
        // that owns the context, as it is for a single access.
        cast_encrypted_operand(&self.node_types, right, target_right, full_payload_domain);

        // Move (not clone) the transformed selector so its NodeKey identity
        // survives for the rules that run after this one; the root comes from the
        // original AST, where it is still the bare column the accessor needs.
        let dummy = Expr::Value(ValueWithSpan {
            value: SqltkValue::Null,
            span: Span::empty(),
        });
        let selector = mem::replace(&mut **target_right, dummy);

        *expr = Self::accessor_call(fn_name, root.clone(), selector);

        Ok(true)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        match node_path.last_1_as::<Expr>() {
            Some((expr @ Expr::BinaryOp { op, .. },)) => {
                Self::accessor_fn(op).is_some() && self.multi_step_chain(expr).is_some()
            }
            _ => false,
        }
    }
}
