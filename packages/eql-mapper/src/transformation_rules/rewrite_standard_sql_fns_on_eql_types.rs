use std::mem;
use std::{collections::HashMap, sync::Arc};

use sqltk::parser::ast::{
    Expr, Function, FunctionArg, FunctionArguments, Ident, ObjectName, ObjectNamePart,
};
use sqltk::{AsNodeKey, NodeKey, NodePath, Visitable};

use crate::{get_sql_function, EqlMapperError, Type, Value};

use super::TransformationRule;

#[derive(Debug)]
pub struct RewriteStandardSqlFnsOnEqlTypes<'ast> {
    node_types: Arc<HashMap<NodeKey<'ast>, Type>>,
}

impl<'ast> RewriteStandardSqlFnsOnEqlTypes<'ast> {
    pub fn new(node_types: Arc<HashMap<NodeKey<'ast>, Type>>) -> Self {
        Self { node_types }
    }

    /// Returns `true` if at least one argument and/or return type is an EQL type.
    fn uses_eql_type(&self, function: &Function) -> bool {
        if matches!(
            self.node_types.get(&function.as_node_key()),
            Some(Type::Value(Value::Eql(_)))
        ) {
            return true;
        }

        match &function.args {
            FunctionArguments::None => false,
            FunctionArguments::Subquery(query) => matches!(
                self.node_types.get(&query.as_node_key()),
                Some(Type::Value(Value::Eql(_)))
            ),
            FunctionArguments::List(list) => list.args.iter().any(|arg| match arg {
                FunctionArg::Named { arg, .. }
                | FunctionArg::ExprNamed { arg, .. }
                | FunctionArg::Unnamed(arg) => matches!(
                    self.node_types.get(&arg.as_node_key()),
                    Some(Type::Value(Value::Eql(_)))
                ),
            }),
        }
    }
}

impl<'ast> TransformationRule<'ast> for RewriteStandardSqlFnsOnEqlTypes<'ast> {
    fn apply<N: Visitable>(
        &mut self,
        node_path: &NodePath<'ast>,
        target_node: &mut N,
    ) -> Result<bool, EqlMapperError> {
        if self.would_edit(node_path, target_node) {
            let function = target_node.downcast_mut::<Function>().unwrap();
            let mut existing_name = mem::take(&mut function.name.0);
            existing_name.insert(0, ObjectNamePart::Identifier(Ident::new("eql_v2")));
            function.name = ObjectName(existing_name);
            return Ok(true);
        }

        Ok(false)
    }

    fn would_edit<N: Visitable>(&mut self, node_path: &NodePath<'ast>, _target_node: &N) -> bool {
        if let Some((_expr, function)) = node_path.last_2_as::<Expr, Function>() {
            return get_sql_function(&function.name).should_rewrite()
                && self.uses_eql_type(function);
        }

        false
    }
}
