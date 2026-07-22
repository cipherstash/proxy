use std::collections::HashMap;
use std::mem;
use std::sync::Arc;

use sqltk::parser::ast::Value as SqltkValue;
use sqltk::parser::ast::{BinaryOperator, Expr, UnaryOperator, ValueWithSpan};
use sqltk::parser::tokenizer::Span;
use sqltk::{NodeKey, NodePath, Visitable};

use crate::unifier::{DomainIdentity, Type, Value};
use crate::EqlMapperError;

use super::helpers::eql_v3_term_call;
use super::TransformationRule;

/// Rewrites `LIKE`/`ILIKE` on encrypted columns into the EQL v3 fuzzy-match form
/// (ADR-0001, ADR-0003):
///
/// - `col LIKE pat`     → `eql_v3.match_term(col) @> eql_v3.match_term(pat)`
/// - `col NOT LIKE pat` → `NOT (eql_v3.match_term(col) @> eql_v3.match_term(pat))`
///
/// Fuzzy match compares bloom-filter terms with `@>` (containment), so unlike a
/// scalar comparison the operator *becomes* `@>` between the two `match_term`
/// calls — mirroring `eql_v3.matches`, whose body is `match_term(a) @> match_term(b)`.
/// A column whose domain stores no bloom filter (`bf`) has no `match_term` and is
/// a capability error.
///
/// `@@` (`BinaryOperator::AtAt`) is the other fuzzy-match surface; it is added
/// once the operator declaration parses it (a separate slice).
#[derive(Debug)]
pub struct RewriteEqlMatchOps<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> RewriteEqlMatchOps<'ast> {
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

    /// The `(encrypted column expr, negated)` of a `LIKE`/`ILIKE` node, if its
    /// left-hand side is an encrypted column.
    fn as_encrypted_like(&self, expr: &'ast Expr) -> Option<(&'ast Expr, bool)> {
        match expr {
            Expr::Like {
                expr, negated, any, ..
            }
            | Expr::ILike {
                expr, negated, any, ..
            } if !*any => {
                let col = &**expr;
                self.eql_identity_of(col).map(|_| (col, *negated))
            }
            _ => None,
        }
    }
}

impl<'ast> TransformationRule<'ast> for RewriteEqlMatchOps<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if !self.would_edit(node_path, target_node) {
            return Ok(false);
        }

        // Read the identity from the ORIGINAL node (node_types is keyed by it).
        let Some((original,)) = node_path.last_1_as::<Expr>() else {
            return Ok(false);
        };
        let Some((col, negated)) = self.as_encrypted_like(original) else {
            return Ok(false);
        };
        let identity = self
            .eql_identity_of(col)
            .expect("checked by as_encrypted_like");

        let Some(term_fn) = identity.match_term_fn() else {
            return Err(EqlMapperError::Transform(format!(
                "encrypted column {} does not support fuzzy match (domain {})",
                identity.token, identity.domain.value
            )));
        };

        let expr = target_node.downcast_mut::<Expr>().unwrap();
        let (col_expr, pat_expr) = match expr {
            Expr::Like { expr, pattern, .. } | Expr::ILike { expr, pattern, .. } => {
                let dummy = Expr::Value(ValueWithSpan {
                    value: SqltkValue::Null,
                    span: Span::empty(),
                });
                let col_expr = mem::replace(&mut **expr, dummy.clone());
                let pat_expr = mem::replace(&mut **pattern, dummy);
                (col_expr, pat_expr)
            }
            _ => return Ok(false),
        };

        // eql_v3.match_term(col) @> eql_v3.match_term(pat)
        let matched = Expr::BinaryOp {
            left: Box::new(eql_v3_term_call(term_fn, col_expr)),
            op: BinaryOperator::AtArrow,
            right: Box::new(eql_v3_term_call(term_fn, pat_expr)),
        };

        *expr = if negated {
            Expr::UnaryOp {
                op: UnaryOperator::Not,
                expr: Box::new(Expr::Nested(Box::new(matched))),
            }
        } else {
            matched
        };

        Ok(true)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((expr,)) = node_path.last_1_as::<Expr>() {
            return self.as_encrypted_like(expr).is_some();
        }
        false
    }
}
