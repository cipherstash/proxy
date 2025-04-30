use std::{collections::HashMap, convert::Infallible, ops::ControlFlow};

use sqltk::{AsNodeKey, Break, NodeKey, Visitable, Visitor};
use sqltk_parser::ast::{
    Expr, Function, FunctionArg, FunctionArgExpr, FunctionArgumentList, FunctionArguments,
    GroupByExpr, ObjectName,
};

use crate::{Type, Value};

/// Checks if `node` has an EQL type (encrypted) and that type is referenced in the `GROUP BY` clause of `select`.
pub(crate) fn is_used_in_group_by_clause<'ast, N: AsNodeKey>(
    node_types: &HashMap<NodeKey<'ast>, Type>,
    group_by: &'ast GroupByExpr,
    node: &'ast N,
) -> bool {
    match node_types.get(&node.as_node_key()) {
        Some(needle @ Type::Value(Value::Eql(_))) => match group_by {
            GroupByExpr::All(_) => true,
            GroupByExpr::Expressions(exprs, _) => {
                let mut visitor = ContainsExprWithType {
                    node_types,
                    ty: needle,
                    found: false,
                };
                exprs.accept(&mut visitor);
                visitor.found
            }
        },
        _ => false,
    }
}

pub(crate) fn wrap_in_1_arg_function(expr: Expr, name: ObjectName) -> Expr {
    Expr::Function(Function {
        name,
        parameters: FunctionArguments::None,
        args: FunctionArguments::List(FunctionArgumentList {
            args: vec![FunctionArg::Unnamed(FunctionArgExpr::Expr(expr))],
            duplicate_treatment: None,
            clauses: vec![],
        }),
        filter: None,
        null_treatment: None,
        over: None,
        within_group: vec![],
        uses_odbc_syntax: false,
    })
}

struct ContainsExprWithType<'ast, 't> {
    node_types: &'t HashMap<NodeKey<'ast>, Type>,
    ty: &'t Type,
    found: bool,
}

impl<'ast> Visitor<'ast> for ContainsExprWithType<'_, 'ast> {
    type Error = Infallible;

    fn enter<N: Visitable>(&mut self, node: &'ast N) -> ControlFlow<Break<Self::Error>> {
        if let Some(expr) = node.downcast_ref::<Expr>() {
            if let Some(expr_ty) = self.node_types.get(&expr.as_node_key()) {
                if expr_ty == self.ty {
                    self.found = true;
                    return ControlFlow::Break(Break::Finished);
                }
            }
        }

        ControlFlow::Continue(())
    }
}
