use sqlparser::ast::{Expr, GroupByExpr, Select, SelectItem};

use crate::{
    inference::{syntactic_eq::SyntacticEq, type_error::TypeError, InferType},
    unifier::{Constructor, Projection, ProjectionColumn, ProjectionColumns, Type},
    TypeInferencer,
};

impl<'ast> InferType<'ast, Select> for TypeInferencer<'ast> {
    fn infer_enter(&mut self, select: &'ast Select) -> Result<(), TypeError> {
        // Every Expr in the SELECT projection
        //  1. that is *not* in the GROUP BY clause
        //  2. that is not already performing aggregation
        // MUST be a ProjectionColumn with `must_be_aggregated: Some(true)`
        let projection_columns: Vec<ProjectionColumn> = match &select.group_by {
            GroupByExpr::Expressions(group_by_exprs, _) => {

                select
                    .projection
                    .iter()
                    .map(|select_item| ProjectionColumn {
                        ty: self.fresh_tvar(),
                        must_be_aggregated: Some(requires_aggregation(select_item, group_by_exprs)),
                        alias: None,
                    })
                    .collect()
            }
            GroupByExpr::All(_) => {
                select
                    .projection
                    .iter()
                    .map(|_| ProjectionColumn {
                        ty: self.fresh_tvar(),
                        must_be_aggregated: Some(false),
                        alias: None,
                    })
                    .collect()
            }
        };

        self.unify_node_with_type(
            &select.projection,
            Type::Constructor(Constructor::Projection(Projection::WithColumns(
                ProjectionColumns(projection_columns),
            )))
            .into_type_cell(),
        )?;

        // TODO: constrain `HAVING` clause

        Ok(())
    }

    fn infer_exit(&mut self, select: &'ast Select) -> Result<(), TypeError> {
        self.unify_nodes(select, &select.projection)?;

        Ok(())
    }
}

fn requires_aggregation(select_item: &SelectItem, group_by_exprs: &[Expr]) -> bool {
    match select_item {
        SelectItem::UnnamedExpr(expr) | SelectItem::ExprWithAlias { expr, alias: _ } => {
            !is_aggregated(expr) && !is_in_group_by(expr, group_by_exprs)
        }

        SelectItem::QualifiedWildcard(_, _) => false,
        SelectItem::Wildcard(_) => false,
    }
}

fn is_aggregated(expr: &Expr) -> bool {
    false
}

fn is_in_group_by(expr: &Expr, group_by_exprs: &[Expr]) -> bool {
    for group_by_expr in group_by_exprs {
        if expr.syntactic_eq(group_by_expr) {
            return true;
        }
    }
    false
}
