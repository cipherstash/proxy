use std::mem;
use std::{collections::HashMap, sync::Arc};

use sqltk::parser::ast::{Expr, Function, Ident, ObjectName};
use sqltk::{AsNodeKey, NodeKey, NodePath, Visitable};

use crate::{
    get_sql_function_def, CompoundIdent, EqlMapperError, RewriteRule, SqlFunction, Type, Value,
};

use super::TransformationRule;

#[derive(Debug)]
pub struct RewriteStandardSqlFnsOnEqlTypes<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> RewriteStandardSqlFnsOnEqlTypes<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }
}

impl<'ast> TransformationRule<'ast> for RewriteStandardSqlFnsOnEqlTypes<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if self.would_edit(node_path, target_node) {
            if let Some((_expr, function)) = node_path.last_2_as::<Expr, Function>() {
                if matches!(
                    self.node_types.get(&function.as_node_key()),
                    Some(Type::Value(Value::Eql(_)))
                ) {
                    let function_name = CompoundIdent::from(&function.name.0);

                    if let Some(SqlFunction {
                        rewrite_rule: RewriteRule::AsEqlFunction,
                        ..
                    }) = get_sql_function_def(&function_name, &function.args)
                    {
                        let function = target_node.downcast_mut::<Function>().unwrap();
                        let mut existing_name = mem::take(&mut function.name.0);
                        existing_name.insert(0, Ident::new("eql_v1"));
                        function.name = ObjectName(existing_name);
                    }
                }
            }
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((_expr, function)) = node_path.last_2_as::<Expr, Function>() {
            if matches!(
                self.node_types.get(&function.as_node_key()),
                Some(Type::Value(Value::Eql(_)))
            ) {
                let function_name = CompoundIdent::from(&function.name.0);

                if let Some(SqlFunction {
                    rewrite_rule: RewriteRule::AsEqlFunction,
                    ..
                }) = get_sql_function_def(&function_name, &function.args)
                {
                    return true;
                }
            }
        }

        false
    }
}
