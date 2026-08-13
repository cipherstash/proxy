use std::collections::HashMap;
use std::mem;
use std::sync::Arc;

use sqltk::parser::ast::{
    DuplicateTreatment, Expr, Function, FunctionArguments, ObjectName, ObjectNamePart,
    Value as SqltkValue,
};
use sqltk::{NodeKey, NodePath, Visitable};

use crate::function_arg::{function_arg_value, function_arg_value_mut};
use crate::unifier::{DomainIdentity, Type, Value};
use crate::EqlMapperError;

use super::helpers::eql_v3_term_call;
use super::TransformationRule;

/// Rewrites `count(DISTINCT enc)` to count the distinct **equality terms**:
///
/// ```sql
/// count(DISTINCT enc)
/// -- becomes
/// count(DISTINCT eql_v3.eq_term(enc))
/// ```
///
/// `DISTINCT` dedupes through the type's default operator class, and for a
/// jsonb-backed domain that compares whole payloads — including `c`, the
/// randomised ciphertext — so every row looks distinct and `count(DISTINCT
/// enc)` silently returns the plain row count. The equality term is
/// deterministic per plaintext, so counting distinct terms counts distinct
/// plaintexts.
///
/// The rewrite is sound for `count` precisely because `count` discards the
/// values it is given: only their multiplicity matters, and the term preserves
/// that. Any other function fed a `DISTINCT` encrypted argument would have its
/// *result* changed by the same substitution (`min(DISTINCT enc)` would return
/// the term, not the ciphertext), so those are rejected outright rather than
/// silently miscomputed.
#[derive(Debug)]
pub struct RewriteEqlAggregateDistinct<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> RewriteEqlAggregateDistinct<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }

    fn eql_identity_of(&self, expr: &'ast Expr) -> Option<DomainIdentity> {
        match self.node_types.get(&NodeKey::new(expr)) {
            Some(Type::Value(Value::Eql(eql_term))) => {
                Some(eql_term.eql_value().domain_identity().clone())
            }
            _ => None,
        }
    }

    /// The encrypted columns among a function's `DISTINCT` arguments,
    /// positionally. Empty when the function takes no `DISTINCT` argument list.
    fn distinct_eql_args(&self, function: &'ast Function) -> Vec<Option<DomainIdentity>> {
        let FunctionArguments::List(list) = &function.args else {
            return vec![];
        };

        if list.duplicate_treatment != Some(DuplicateTreatment::Distinct) {
            return vec![];
        }

        list.args
            .iter()
            .map(|arg| function_arg_value(arg).and_then(|expr| self.eql_identity_of(expr)))
            .collect()
    }

    /// Whether `name` is the built-in `count` — unqualified, or explicitly
    /// qualified with `pg_catalog`. Matching on the last identifier alone would
    /// also catch a user's `custom_schema.count(...)`, which is a different
    /// function with unknown semantics.
    fn is_builtin_count(name: &ObjectName) -> bool {
        let bare = match &name.0[..] {
            [ObjectNamePart::Identifier(bare)] => bare,
            [ObjectNamePart::Identifier(schema), ObjectNamePart::Identifier(bare)]
                if schema.value.eq_ignore_ascii_case("pg_catalog") =>
            {
                bare
            }
            _ => return false,
        };

        bare.value.eq_ignore_ascii_case("count")
    }
}

impl<'ast> TransformationRule<'ast> for RewriteEqlAggregateDistinct<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        // Read the identities from the ORIGINAL function — `node_types` is
        // keyed by it, and the target's children are already rewritten.
        let Some((_expr, original)) = node_path.last_2_as::<Expr, Function>() else {
            return Ok(false);
        };

        let distinct_args = self.distinct_eql_args(original);
        if distinct_args.iter().all(Option::is_none) {
            return Ok(false);
        }

        if !Self::is_builtin_count(&original.name) {
            return Err(EqlMapperError::Transform(format!(
                "DISTINCT with an encrypted argument is not supported in {}(...); only count(DISTINCT ...) is",
                original.name
            )));
        }

        let Some(target) = target_node.downcast_mut::<Function>() else {
            return Ok(false);
        };

        let FunctionArguments::List(list) = &mut target.args else {
            return Ok(false);
        };

        for (arg, identity) in list.args.iter_mut().zip(distinct_args.iter()) {
            let Some(identity) = identity else { continue };

            let Some(term_fn) = identity.eq_term_fn() else {
                return Err(EqlMapperError::Transform(format!(
                    "encrypted column {} cannot be counted with DISTINCT (domain {} carries no equality term)",
                    identity.token, identity.domain.value
                )));
            };

            if let Some(expr) = function_arg_value_mut(arg) {
                let counted = mem::replace(expr, Expr::Value(SqltkValue::Null.into()));
                *expr = eql_v3_term_call(term_fn, counted);
            }
        }

        Ok(true)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        match node_path.last_2_as::<Expr, Function>() {
            Some((_expr, original)) => self.distinct_eql_args(original).iter().any(Option::is_some),
            None => false,
        }
    }
}
