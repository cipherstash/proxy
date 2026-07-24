use std::{any::type_name, collections::HashMap, sync::Arc};

use sqltk::parser::ast::{Expr, Value, ValueWithSpan};
use sqltk::parser::tokenizer::Span;
use sqltk::{NodeKey, NodePath, Visitable};

use crate::unifier::{EqlTerm, Type, Value as UnifierValue};
use crate::EqlMapperError;

use super::helpers::{cast_to_v3_domain, v3_cast_target};
use super::TransformationRule;

#[derive(Debug)]
pub struct CastLiteralsAsEncrypted<'ast> {
    encrypted_literals: HashMap<NodeKey<'ast>, Value>,
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> CastLiteralsAsEncrypted<'ast> {
    pub fn new(
        encrypted_literals: HashMap<NodeKey<'ast>, Value>,
        node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
    ) -> Self {
        Self {
            encrypted_literals,
            node_types,
        }
    }
}

impl<'ast> TransformationRule<'ast> for CastLiteralsAsEncrypted<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if self.would_edit(node_path, target_node) {
            if let Some((original @ Expr::Value(ValueWithSpan { value, .. }),)) =
                node_path.last_1_as::<Expr>()
            {
                if let Some(replacement) = self.encrypted_literals.remove(&NodeKey::new(value)) {
                    // The literal's domain identity determines the cast target; its
                    // role (query operand vs stored value) determines which v3
                    // domain (query twin vs column domain).
                    let Some(Type::Value(UnifierValue::Eql(eql_term))) =
                        self.node_types.get(&NodeKey::new(original))
                    else {
                        return Err(EqlMapperError::Transform(format!(
                            "{}: encrypted literal has no EQL type",
                            type_name::<Self>()
                        )));
                    };

                    let target_node = target_node.downcast_mut::<Expr>().unwrap();
                    *target_node = match eql_term {
                        // A JSON selector — the RHS of `->`/`->>`, or the path
                        // argument of `jsonb_path_query` — is passed to the EQL v3
                        // function as the encrypted-selector *text*
                        // (`eql_v3."->"(json, text)`, `jsonb_path_query(json,
                        // text)`), not a jsonb-domain payload. The encrypt pipeline
                        // produces that selector text (a SteVec `QueryOp::SteVecSelector`
                        // token), so it is emitted verbatim, uncast.
                        EqlTerm::JsonAccessor(_) | EqlTerm::JsonPath(_) => {
                            Expr::Value(ValueWithSpan {
                                value: replacement,
                                span: Span::empty(),
                            })
                        }
                        _ => {
                            let identity = eql_term.eql_value().domain_identity().clone();
                            let (schema, domain) = v3_cast_target(node_path, &identity);
                            cast_to_v3_domain(replacement, &schema, &domain)
                        }
                    };
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((Expr::Value(ValueWithSpan { value, .. }),)) = node_path.last_1_as::<Expr>() {
            return self.encrypted_literals.contains_key(&NodeKey::new(value));
        }
        false
    }

    fn check_postcondition(&self) -> Result<(), EqlMapperError> {
        if self.encrypted_literals.is_empty() {
            Ok(())
        } else {
            Err(EqlMapperError::Transform(format!(
                "Postcondition failed in {}: unused encrypted literals",
                type_name::<Self>()
            )))
        }
    }
}
