use std::collections::HashMap;
use std::sync::Arc;

use sqltk::parser::ast::{Assignment, Expr, Function, FunctionArguments, Values};
use sqltk::{NodeKey, NodePath, Visitable};

use crate::function_arg::{function_arg_value, function_arg_value_mut};
use crate::unifier::{Type, Value};
use crate::EqlMapperError;

use super::helpers::{cast_encrypted_operand, full_payload_domain};
use super::TransformationRule;

/// Casts encrypted values that must carry the column's **whole payload** — the
/// ciphertext plus every search term the column indexes — to the column domain,
/// rather than to a term-only `eql_v3.query_*` twin.
///
/// Three contexts need it, and none of them is a predicate whose own rewrite
/// rule could own the cast:
///
/// - `INSERT INTO t (col) VALUES ($1)` — the value is stored.
/// - `UPDATE t SET col = 'x'` — likewise.
/// - `eql_v3.jsonb_contains(col, $1)` and friends — a containment needle is a
///   whole document, and the cast is what lets PostgreSQL use the GIN index over
///   `eql_v3.jsonb_array(col)`. Clients on platforms without operator support
///   (Supabase, PostgREST) write these function forms directly, so they arrive
///   already spelled `eql_v3.*` with no operator for a rewrite rule to catch.
///
/// The rule fires on the enclosing `Values`, `Assignment` and `Function` nodes
/// rather than on the value expressions, so "this operand carries a full
/// payload" is a fact about the construct that owns it, not a guess made by
/// walking up the tree from a literal.
///
/// A JSON selector argument is left uncast — [`full_payload_domain`] returns
/// `None` for it, because `eql_v3.jsonb_path_query(json, text)` takes the bare
/// encrypted selector text.
#[derive(Debug)]
pub struct CastFullPayloadOperands<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> CastFullPayloadOperands<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }

    /// Whether `expr` is an encrypted literal or placeholder that this rule
    /// would cast. Shared by `apply` and `would_edit` so the dry run agrees with
    /// the real run.
    fn needs_cast(&self, expr: &'ast Expr) -> bool {
        matches!(expr, Expr::Value(_))
            && matches!(
                self.node_types.get(&NodeKey::new(expr)),
                Some(Type::Value(Value::Eql(eql_term))) if full_payload_domain(eql_term).is_some()
            )
    }

    /// The argument expressions of a function call, in order.
    fn args(function: &Function) -> impl Iterator<Item = &Expr> {
        let args = match &function.args {
            FunctionArguments::List(list) => Some(list.args.iter()),
            _ => None,
        };

        args.into_iter().flatten().filter_map(function_arg_value)
    }

    fn args_mut(function: &mut Function) -> impl Iterator<Item = &mut Expr> {
        let args = match &mut function.args {
            FunctionArguments::List(list) => Some(list.args.iter_mut()),
            _ => None,
        };

        args.into_iter()
            .flatten()
            .filter_map(function_arg_value_mut)
    }

    /// Whether `function` is an `eql_v3.*` call — the only functions whose
    /// encrypted arguments this rule owns.
    fn is_eql_v3_function(function: &Function) -> bool {
        function
            .name
            .0
            .first()
            .and_then(|part| part.as_ident())
            .is_some_and(|ident| ident.value.eq_ignore_ascii_case("eql_v3"))
    }
}

impl<'ast> TransformationRule<'ast> for CastFullPayloadOperands<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if let Some((original,)) = node_path.last_1_as::<Values>() {
            let Some(target) = target_node.downcast_mut::<Values>() else {
                return Ok(false);
            };

            let mut edited = false;
            for (original_row, target_row) in original.rows.iter().zip(target.rows.iter_mut()) {
                for (original_expr, target_expr) in original_row.iter().zip(target_row.iter_mut()) {
                    edited |= cast_encrypted_operand(
                        &self.node_types,
                        original_expr,
                        target_expr,
                        full_payload_domain,
                    );
                }
            }

            return Ok(edited);
        }

        if let Some((original,)) = node_path.last_1_as::<Assignment>() {
            let Some(target) = target_node.downcast_mut::<Assignment>() else {
                return Ok(false);
            };

            return Ok(cast_encrypted_operand(
                &self.node_types,
                &original.value,
                &mut target.value,
                full_payload_domain,
            ));
        }

        if let Some((original,)) = node_path.last_1_as::<Function>() {
            if !Self::is_eql_v3_function(original) {
                return Ok(false);
            }

            let Some(target) = target_node.downcast_mut::<Function>() else {
                return Ok(false);
            };

            let mut edited = false;
            for (original_arg, target_arg) in Self::args(original).zip(Self::args_mut(target)) {
                edited |= cast_encrypted_operand(
                    &self.node_types,
                    original_arg,
                    target_arg,
                    full_payload_domain,
                );
            }

            return Ok(edited);
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((original,)) = node_path.last_1_as::<Values>() {
            return original
                .rows
                .iter()
                .flat_map(|row| row.iter())
                .any(|expr| self.needs_cast(expr));
        }

        if let Some((original,)) = node_path.last_1_as::<Assignment>() {
            return self.needs_cast(&original.value);
        }

        if let Some((original,)) = node_path.last_1_as::<Function>() {
            return Self::is_eql_v3_function(original)
                && Self::args(original).any(|expr| self.needs_cast(expr));
        }

        false
    }
}
