use super::helpers::{cast_to_v3_domain, v3_cast_target};
use super::TransformationRule;
use crate::unifier::{EqlTerm, Type, Value as UnifierValue};
use crate::EqlMapperError;
use sqltk::parser::ast::{Expr, Value, ValueWithSpan};
use sqltk::parser::tokenizer::Span;
use sqltk::{NodeKey, NodePath, Visitable};
use std::collections::HashMap;
use std::sync::Arc;

#[derive(Debug)]
pub struct CastParamsAsEncrypted<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> CastParamsAsEncrypted<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }
}

impl<'ast> TransformationRule<'ast> for CastParamsAsEncrypted<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if self.would_edit(node_path, target_node) {
            // Resolve the operand's v3 cast target from its domain identity and
            // its role (query operand → query twin; stored value → column domain).
            let Some((original,)) = node_path.last_1_as::<Expr>() else {
                return Ok(false);
            };
            let Some(Type::Value(UnifierValue::Eql(eql_term))) =
                self.node_types.get(&NodeKey::new(original))
            else {
                return Ok(false);
            };

            // A JSON selector operand (RHS of `->`/`->>`, or the path argument of
            // `jsonb_path_query`) is passed to the EQL v3 function as the encrypted
            // selector *text* — `eql_v3."->"(json, text)`, `jsonb_path_query(json,
            // text)` — not a jsonb query-domain payload. The proxy encrypts these
            // params as SteVec selectors, so the placeholder is left bare (its type
            // is inferred from the function signature) rather than cast to the
            // `eql_v3.query_*` twin. Mirrors the JsonAccessor arm of
            // `CastLiteralsAsEncrypted`.
            if matches!(eql_term, EqlTerm::JsonAccessor(_) | EqlTerm::JsonPath(_)) {
                return Ok(false);
            }

            let identity = eql_term.eql_value().domain_identity().clone();
            let (schema, domain) = v3_cast_target(node_path, &identity);

            if let Some(
                expr @ Expr::Value(ValueWithSpan {
                    value: Value::Placeholder(_),
                    ..
                }),
            ) = target_node.downcast_mut()
            {
                let to_wrap = std::mem::replace(
                    expr,
                    Expr::Value(ValueWithSpan {
                        value: Value::Null,
                        span: Span::empty(),
                    }),
                );
                let Expr::Value(ValueWithSpan {
                    value: value @ Value::Placeholder(_),
                    ..
                }) = to_wrap
                else {
                    unreachable!("the Expr is known to be Expr::Value(ValueWithSpan::{{ value: Value::Placeholder(_), .. }})")
                };

                *expr = cast_to_v3_domain(value, &schema, &domain);
                return Ok(true);
            }
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((
            node @ Expr::Value(ValueWithSpan {
                value: Value::Placeholder(_),
                ..
            }),
        )) = node_path.last_1_as()
        {
            if let Some(Type::Value(crate::unifier::Value::Eql(_))) =
                self.node_types.get(&NodeKey::new(node))
            {
                return true;
            }
        }
        false
    }
}
