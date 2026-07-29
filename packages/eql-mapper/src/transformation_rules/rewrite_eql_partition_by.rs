use std::collections::HashMap;
use std::mem;
use std::sync::Arc;

use sqltk::parser::ast::{Expr, Value as SqltkValue, WindowType};
use sqltk::{NodeKey, NodePath, Visitable};

use crate::unifier::{DomainIdentity, Type, Value};
use crate::EqlMapperError;

use super::helpers::eql_v3_term_call;
use super::TransformationRule;

/// Rewrites `PARTITION BY` on an encrypted column to partition by its
/// **equality term**:
///
/// ```sql
/// rank() OVER (PARTITION BY enc)
/// -- becomes
/// rank() OVER (PARTITION BY eql_v3.eq_term(enc))
/// ```
///
/// Partitioning groups rows by equality, and like `GROUP BY` and `DISTINCT` it
/// goes through the type's default operator class rather than EQL's `=`
/// overload. For a jsonb-backed domain that compares whole payloads, including
/// `c`, the randomised ciphertext — so every row lands in its own partition and
/// every window function silently sees a partition of one.
///
/// The window's own `ORDER BY` needs no handling here:
/// [`super::RewriteEqlOrderBy`] matches on `OrderByExpr` wherever it appears,
/// including inside a window specification.
#[derive(Debug)]
pub struct RewriteEqlPartitionBy<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> RewriteEqlPartitionBy<'ast> {
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

    /// The `PARTITION BY` expressions of a function's window, if it has one.
    fn partition_by(expr: &Expr) -> Option<&Vec<Expr>> {
        let Expr::Function(function) = expr else {
            return None;
        };

        match function.over.as_ref()? {
            WindowType::WindowSpec(spec) => Some(&spec.partition_by),
            WindowType::NamedWindow(_) => None,
        }
    }

    /// The encrypted columns a window partitions on, positionally.
    fn partitioned_identities(&self, expr: &'ast Expr) -> Vec<Option<DomainIdentity>> {
        Self::partition_by(expr)
            .map(|exprs| exprs.iter().map(|e| self.eql_identity_of(e)).collect())
            .unwrap_or_default()
    }
}

impl<'ast> TransformationRule<'ast> for RewriteEqlPartitionBy<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        // Read the identities from the ORIGINAL expression — `node_types` is
        // keyed by it, and the target's children are already rewritten.
        let Some((original,)) = node_path.last_1_as::<Expr>() else {
            return Ok(false);
        };

        let partitioned = self.partitioned_identities(original);
        if partitioned.iter().all(Option::is_none) {
            return Ok(false);
        }

        let Some(Expr::Function(target)) = target_node.downcast_mut::<Expr>() else {
            return Ok(false);
        };

        let Some(WindowType::WindowSpec(spec)) = target.over.as_mut() else {
            return Ok(false);
        };

        for (expr, identity) in spec.partition_by.iter_mut().zip(partitioned.iter()) {
            let Some(identity) = identity else { continue };

            let Some(term_fn) = identity.eq_term_fn() else {
                return Err(EqlMapperError::Transform(format!(
                    "encrypted column {} cannot be used in PARTITION BY (domain {} carries no equality term)",
                    identity.token, identity.domain.value
                )));
            };

            let partitioned = mem::replace(expr, Expr::Value(SqltkValue::Null.into()));
            *expr = eql_v3_term_call(term_fn, partitioned);
        }

        Ok(true)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        match node_path.last_1_as::<Expr>() {
            Some((original,)) => self
                .partitioned_identities(original)
                .iter()
                .any(Option::is_some),
            None => false,
        }
    }
}
