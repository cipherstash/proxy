use std::{any::type_name, collections::HashMap};

use sqltk::parser::ast::{
    CastKind, DataType, Expr, Function, FunctionArg, FunctionArgExpr, FunctionArgumentList,
    FunctionArguments, Ident, ObjectName, Value,
};
use sqltk::{NodeKey, NodePath, Visitable};

use crate::EqlMapperError;

use super::TransformationRule;

#[derive(Debug)]
pub struct ReplacePlaintextEqlLiterals<'ast> {
    encrypted_literals: HashMap<NodeKey<'ast>, Value>,
}

impl<'ast> ReplacePlaintextEqlLiterals<'ast> {
    pub fn new(encrypted_literals: HashMap<NodeKey<'ast>, Value>) -> Self {
        Self { encrypted_literals }
    }
}

impl<'ast> TransformationRule<'ast> for ReplacePlaintextEqlLiterals<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if self.would_edit(node_path, target_node) {
            if let Some((Expr::Value(value),)) = node_path.last_1_as::<Expr>() {
                if let Some(replacement) = self.encrypted_literals.remove(&NodeKey::new(value)) {
                    let target_node = target_node.downcast_mut::<Expr>().unwrap();
                    *target_node = make_row_expression(replacement);
                    return Ok(true);
                }
            }
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((Expr::Value(value),)) = node_path.last_1_as::<Expr>() {
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

fn make_row_expression(replacement: Value) -> Expr {
    Expr::Function(Function {
        name: ObjectName(vec![Ident::new("ROW")]),
        uses_odbc_syntax: false,
        parameters: FunctionArguments::None,
        args: FunctionArguments::List(FunctionArgumentList {
            duplicate_treatment: None,
            clauses: vec![],
            args: vec![FunctionArg::Unnamed(FunctionArgExpr::Expr(Expr::Cast {
                kind: CastKind::DoubleColon,
                expr: Box::new(Expr::Value(replacement)),
                data_type: DataType::JSONB,
                format: None,
            }))],
        }),
        filter: None,
        null_treatment: None,
        over: None,
        within_group: vec![],
    })
}
