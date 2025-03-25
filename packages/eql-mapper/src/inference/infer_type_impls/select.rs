use std::mem;

use sqlparser::ast::{Expr, GroupByExpr, Select, SelectItem};

use crate::{
    inference::{type_error::TypeError, InferType},
    unifier::{Constructor, Projection, ProjectionColumn, ProjectionColumns, Type},
    SqlIdent, TypeInferencer,
};

impl<'ast> InferType<'ast, Select> for TypeInferencer<'ast> {
    fn infer_enter(&mut self, select: &'ast Select) -> Result<(), TypeError> {
        // TODO: examine `GROUP BY` clause and check which columns require aggregation.

        if let GroupByExpr::Expressions(group_by_exprs, _) = &select.group_by {
            // Every Expr in the SELECT projection
            //  1. that is *not* in the GROUP BY clause
            //  2. that is not already performing aggregation
            // MUST be a ProjectionColumn with `must_be_aggregated: true`

            let projection_columns: Vec<ProjectionColumn> = select
                .projection
                .iter()
                .map(|select_item| ProjectionColumn {
                    // TODO: How do we deal with wildcard projections?
                    ty: self.fresh_tvar(),
                    must_be_aggregated: requires_aggregation(select_item, group_by_exprs),
                    alias: None,
                })
                .collect();

            self.unify_node_with_type(
                &select.projection,
                Type::Constructor(Constructor::Projection(Projection::WithColumns(
                    ProjectionColumns(projection_columns),
                )))
                .into_type_cell(),
            )?;
        }

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

        SelectItem::QualifiedWildcard(object_name, wildcard_additional_options) => todo!(),
        SelectItem::Wildcard(wildcard_additional_options) => todo!(),
    }
}

fn is_aggregated(expr: &Expr) -> bool {
    false
}

fn is_in_group_by(expr: &Expr, group_by_exprs: &[Expr]) -> bool {
    for group_by_expr in group_by_exprs {
        if ExprEq(expr) == ExprEq(group_by_expr) {
            return true;
        }
    }
    false
}


/// Newtype wrapper around AST nodes that implements `PartialEq` such that:
/// - `Expr::Identifier` and `Expr::CompoundIdentifier` are compared by their `SqlIdent` values.
/// - `Expr::Nested` is compared by ignoring the differences in nesting.
/// - `Query` is compared as normal except for recursively contained `Expr` nodes.
struct ExprEq<'a>(&'a Expr);

impl<'a> PartialEq for ExprEq<'a> {
    fn eq(&self, other: &Self) -> bool {
        // Expr::Nested(_) requires special handling because the parens are superfluous when it comes to equality.
        match (self.0, other.0) {
            (Expr::Nested(expr_left), expr_right) => {
                return ExprEq(expr_left) == ExprEq(expr_right)
            }
            (expr_left, Expr::Nested(expr_right)) => {
                return ExprEq(expr_left) == ExprEq(expr_right)
            }
            _ => {}
        }

        // If the discriminants are different then the nodes are different, so we can bail out.
        if mem::discriminant(self.0) != mem::discriminant(other.0) {
            return false;
        }

        // Otherwise, we need to compare the cases when both discriminants are the same.
        match (self.0, other.0) {
            (Expr::Identifier(ident_left), Expr::Identifier(ident_right)) => {
                SqlIdent(ident_left) == SqlIdent(ident_right)
            }
            (Expr::CompoundIdentifier(idents_left), Expr::CompoundIdentifier(idents_right)) => {
                SqlIdent(idents_left.last().unwrap()) == SqlIdent(idents_right.last().unwrap())
            }
            (
                Expr::JsonAccess {
                    value: value_left,
                    path: path_left,
                },
                Expr::JsonAccess { .. },
            ) => left == right,
            (left @ Expr::CompositeAccess { .. }, right @ Expr::CompositeAccess { .. }) => {
                left == right
            }
            (Expr::IsFalse(expr_left), Expr::IsFalse(expr_right))
            | (Expr::IsNotFalse(expr_left), Expr::IsNotFalse(expr_right))
            | (Expr::IsTrue(expr_left), Expr::IsTrue(expr_right))
            | (Expr::IsNotTrue(expr_left), Expr::IsNotTrue(expr_right))
            | (Expr::IsNull(expr_left), Expr::IsNull(expr_right))
            | (Expr::IsNotNull(expr_left), Expr::IsNotNull(expr_right))
            | (Expr::IsUnknown(expr_left), Expr::IsUnknown(expr_right))
            | (Expr::IsNotUnknown(expr_left), Expr::IsNotUnknown(expr_right)) => {
                ExprEq(expr_left) == ExprEq(expr_right)
            }
            (
                Expr::IsDistinctFrom(expr_left, expr1_left),
                Expr::IsDistinctFrom(expr_right, expr1_right),
            )
            | (
                Expr::IsNotDistinctFrom(expr_left, expr1_left),
                Expr::IsNotDistinctFrom(expr_right, expr1_right),
            ) => {
                ExprEq(expr_left) == ExprEq(expr_right) && ExprEq(expr1_left) == ExprEq(expr1_right)
            }
            (
                Expr::InList {
                    expr: expr_left,
                    list: list_left,
                    negated: negated_left,
                },
                Expr::InList {
                    expr: expr_right,
                    list: list_right,
                    negated: negated_right,
                },
            ) => {
                ExprEq(expr_left) == ExprEq(expr_right)
                    && list_left
                        .iter()
                        .zip(list_right.iter())
                        .all(|(l, r)| ExprEq(l) == ExprEq(r))
                    && negated_left == negated_right
            }
            (
                Expr::InSubquery {
                    expr: expr_left,
                    subquery: subquery_left,
                    negated: negated_left,
                },
                Expr::InSubquery {
                    expr: expr_right,
                    subquery: subquery_right,
                    negated: negated_right,
                },
            ) => {
                ExprEq(expr_left) == ExprEq(expr_right)
                    && ExprEq(subquery_left) == ExprEq(subquery_right)
                    && negated_left == negated_right
            }
            (
                Expr::InUnnest {
                    expr: expr_left,
                    array_expr: array_expr_left,
                    negated: negated_left,
                },
                Expr::InUnnest {
                    expr: expr_right,
                    array_expr: array_expr_right,
                    negated: negated_right,
                },
            ) => {
                ExprEq(expr_left) == ExprEq(expr_right)
                    && ExprEq(array_expr_left) == ExprEq(array_expr_right)
                    && negated_left == negated_right
            }
            (
                Expr::Between {
                    expr: expr_left,
                    negated: negated_left,
                    low: low_left,
                    high: high_left,
                },
                Expr::Between {
                    expr: expr_right,
                    negated: negated_right,
                    low: low_right,
                    high: high_right,
                },
            ) => {
                ExprEq(expr_left) == ExprEq(expr_right)
                    && negated_left == negated_right
                    && ExprEq(low_left) == ExprEq(low_right)
                    && ExprEq(high_left) == ExprEq(high_right)
            }
            (
                Expr::BinaryOp {
                    left: left_left,
                    op: op_left,
                    right: right_left,
                },
                Expr::BinaryOp {
                    left: left_right,
                    op: op_right,
                    right: right_right,
                },
            ) => {
                ExprEq(left_left) == ExprEq(left_right)
                    && ExprEq(right_left) == ExprEq(right_right)
                    && op_left == op_right
            }
            (
                Expr::Like {
                    negated: negated_left,
                    any: any_left,
                    expr: expr_left,
                    pattern: pattern_left,
                    escape_char: escape_char_left,
                },
                Expr::Like {
                    negated: negated_right,
                    any: any_right,
                    expr: expr_right,
                    pattern: pattern_right,
                    escape_char: escape_char_right,
                },
            )
            | (
                Expr::ILike {
                    negated: negated_left,
                    any: any_left,
                    expr: expr_left,
                    pattern: pattern_left,
                    escape_char: escape_char_left,
                },
                Expr::ILike {
                    negated: negated_right,
                    any: any_right,
                    expr: expr_right,
                    pattern: pattern_right,
                    escape_char: escape_char_right,
                },
            ) => {
                negated_left == negated_right
                    && any_left == any_right
                    && ExprEq(expr_left) == ExprEq(expr_right)
                    && ExprEq(pattern_left) == ExprEq(pattern_right)
                    && escape_char_left == escape_char_right
            }
            (
                Expr::SimilarTo {
                    negated: negated_left,
                    expr: expr_left,
                    pattern: pattern_left,
                    escape_char: escape_char_left,
                },
                Expr::SimilarTo {
                    negated: negated_right,
                    expr: expr_right,
                    pattern: pattern_right,
                    escape_char: escape_char_right,
                },
            ) => {
                negated_left == negated_right
                    && ExprEq(expr_left) == ExprEq(expr_right)
                    && ExprEq(pattern_left) == ExprEq(pattern_right)
                    && escape_char_left == escape_char_right
            }
            (
                Expr::InSubquery {
                    expr: expr_left,
                    subquery: subquery_left,
                    negated: negated_left,
                },
                Expr::InSubquery {
                    expr: expr_right,
                    subquery: subquery_right,
                    negated: negated_right,
                },
            ) => {
                ExprEq(expr_left) == ExprEq(expr_right)
                    && ExprEq(subquery_left) == ExprEq(subquery_right)
                    && negated_left == negated_right
            }
            (
                Expr::InUnnest {
                    expr: expr_left,
                    array_expr: array_expr_left,
                    negated: negated_left,
                },
                Expr::InUnnest {
                    expr: expr_right,
                    array_expr: array_expr_right,
                    negated: negated_right,
                },
            ) => {
                ExprEq(expr_left) == ExprEq(expr_right)
                    && ExprEq(array_expr_left) == ExprEq(array_expr_right)
                    && negated_left == negated_right
            }
            (
                Expr::Between {
                    expr: expr_left,
                    negated: negated_left,
                    low: low_left,
                    high: high_left,
                },
                Expr::Between {
                    expr: expr_right,
                    negated: negated_right,
                    low: low_right,
                    high: high_right,
                },
            ) => {
                ExprEq(expr_right) == ExprEq(expr_left)
                    && negated_left == negated_right
                    && ExprEq(low_left) == ExprEq(low_right)
                    && ExprEq(high_left) == ExprEq(high_right)
            }
            (
                Expr::RLike {
                    negated: negated_left,
                    expr: expr_left,
                    pattern: pattern_left,
                    regexp: regexp_left,
                },
                Expr::RLike {
                    negated: negated_right,
                    expr: expr_right,
                    pattern: pattern_right,
                    regexp: regexp_right,
                },
            ) => {
                negated_left == negated_right
                    && ExprEq(expr_left) == ExprEq(expr_right)
                    && ExprEq(pattern_left) == ExprEq(pattern_right)
                    && regexp_left == regexp_right
            }
            (
                Expr::AnyOp {
                    left: left_left,
                    compare_op: compare_op_left,
                    right: right_left,
                    is_some: is_some_left,
                },
                Expr::AnyOp {
                    left: left_right,
                    compare_op: compare_op_right,
                    right: right_right,
                    is_some: is_some_right,
                },
            ) => {
                ExprEq(left_left) == ExprEq(left_right)
                    && compare_op_left == compare_op_right
                    && ExprEq(right_left) == ExprEq(right_right)
                    && is_some_left == is_some_right
            }
            (
                Expr::AllOp {
                    left: left_left,
                    compare_op: compare_op_left,
                    right: right_left,
                },
                Expr::AllOp {
                    left: left_right,
                    compare_op: compare_op_right,
                    right: right_right,
                },
            ) => {
                ExprEq(left_left) == ExprEq(left_right)
                    && compare_op_left == compare_op_right
                    && ExprEq(right_left) == ExprEq(right_right)
            }
            (
                Expr::UnaryOp {
                    op: op_left,
                    expr: expr_left,
                },
                Expr::UnaryOp {
                    op: op_right,
                    expr: expr_right,
                },
            ) => op_left == op_right && ExprEq(expr_left) == ExprEq(expr_right),
            (
                Expr::Convert {
                    is_try: is_try_left,
                    expr: expr_left,
                    data_type: data_type_left,
                    charset: charset_left,
                    target_before_value: target_before_value_left,
                    styles: styles_left,
                },
                Expr::Convert {
                    is_try: is_try_right,
                    expr: expr_right,
                    data_type: data_type_right,
                    charset: charset_right,
                    target_before_value: target_before_value_right,
                    styles: styles_right,
                },
            ) => {
                is_try_left == is_try_right
                    && ExprEq(expr_left) == ExprEq(expr_right)
                    && data_type_left == data_type_right
                    && ExprEq(charset_left) == ExprEq(charset_right)
                    && target_before_value_left == target_before_value_right
                    && ExprEq(styles_left) == ExprEq(styles_right)
            }
            (
                Expr::Cast {
                    kind: kind_left,
                    expr: expr_left,
                    data_type: data_type_left,
                    format: format_left,
                },
                Expr::Cast {
                    kind: kind_right,
                    expr: expr_right,
                    data_type: data_type_right,
                    format: format_right,
                },
            ) => {
                kind_left == kind_right
                    && ExprEq(expr_left) == ExprEq(expr_right)
                    && data_type_left == data_type_right
                    && format_right == format_left
            }
            (
                Expr::AtTimeZone {
                    timestamp: timestamp_left,
                    time_zone: time_zone_left,
                },
                Expr::AtTimeZone {
                    timestamp: timestamp_right,
                    time_zone: time_zone_right,
                },
            ) => {
                ExprEq(timestamp_left) == ExprEq(timestamp_right)
                    && ExprEq(time_zone_left) == ExprEq(time_zone_right)
            }
            (
                Expr::Extract {
                    field: field_left,
                    syntax: syntax_left,
                    expr: expr_left,
                },
                Expr::Extract {
                    field: field_right,
                    syntax: syntax_right,
                    expr: expr_right,
                },
            ) => {
                field_left == field_right
                    && syntax_left == syntax_right
                    && ExprEq(expr_left) == ExprEq(expr_right)
            }
            (
                Expr::Ceil {
                    expr: expr_left,
                    field: field_left,
                },
                Expr::Ceil {
                    expr: expr_right,
                    field: field_right,
                },
            )
            | (
                Expr::Floor {
                    expr: expr_left,
                    field: field_left,
                },
                Expr::Floor {
                    expr: expr_right,
                    field: field_right,
                },
            ) => ExprEq(expr_left) == ExprEq(expr_right) && field_left == field_right,
            (
                Expr::Position {
                    expr: expr_left,
                    r#in: in_left,
                },
                Expr::Position {
                    expr: expr_right,
                    r#in: in_right,
                },
            ) => ExprEq(expr_left) == ExprEq(expr_right) && ExprEq(in_left) == ExprEq(in_right),
            (
                Expr::Substring {
                    expr: expr_left,
                    substring_from: substring_from_left,
                    substring_for: substring_for_left,
                    special: special_left,
                },
                Expr::Substring {
                    expr: expr_right,
                    substring_from: substring_from_right,
                    substring_for: substring_for_right,
                    special: special_right,
                },
            ) => {
                ExprEq(expr_left) == ExprEq(expr_right)
                    && ExprEq(substring_from_left) == ExprEq(substring_from_right)
                    && ExprEq(substring_for_left) == ExprEq(substring_for_right)
                    && special_left == special_right
            }
            (
                Expr::Trim {
                    expr: expr_left,
                    trim_where: trim_where_left,
                    trim_what: trim_what_left,
                    trim_characters: trim_characters_left,
                },
                Expr::Trim {
                    expr: expr_right,
                    trim_where: trim_where_right,
                    trim_what: trim_what_right,
                    trim_characters: trim_characters_right,
                },
            ) => {
                ExprEq(expr_left) == ExprEq(expr_right)
                    && ExprEq(trim_where_left) == ExprEq(trim_where_right)
                    && ExprEq(trim_what_left) == ExprEq(trim_what_right)
                    && ExprEq(trim_characters_left) == ExprEq(trim_characters_right)
            }
            (
                Expr::Overlay {
                    expr: expr_left,
                    overlay_what: overlay_what_left,
                    overlay_from: overlay_from_left,
                    overlay_for: overlay_for_left,
                },
                Expr::Overlay {
                    expr: expr_right,
                    overlay_what: overlay_what_right,
                    overlay_from: overlay_from_right,
                    overlay_for: overlay_for_right,
                },
            ) => {
                ExprEq(expr_left) == ExprEq(expr_right)
                    && ExprEq(overlay_what_left) == ExprEq(overlay_what_right)
                    && ExprEq(overlay_from_left) == ExprEq(overlay_from_right)
                    && ExprEq(overlay_for_left) == ExprEq(overlay_for_right)
            }
            (
                Expr::Collate {
                    expr: expr_left,
                    collation: collation_left,
                },
                Expr::Collate {
                    expr: expr_right,
                    collation: collation_right,
                },
            ) => {
                ExprEq(expr_left) == ExprEq(expr_right)
                    && Expr(collation_left) == ExprEq(collation_right)
            }
            (Expr::Value(value_left), Expr::Value(value_right)) => {
                ExprEq(value_left) == ExprEq(value_right)
            }
            (
                Expr::IntroducedString {
                    introducer: introducer_left,
                    value: value_left,
                },
                Expr::IntroducedString {
                    introducer: introducer_right,
                    value: value_right,
                },
            ) => introducer_left == introducer_right && ExprEq(value_left) == ExprEq(value_right),
            (
                Expr::TypedString {
                    data_type: data_type_left,
                    value: value_left,
                },
                Expr::TypedString {
                    data_type: data_type_right,
                    value: value_right,
                },
            ) => data_type_left == data_type_right && ExprEq(value_left) == ExprEq(value_right),
            (
                Expr::MapAccess {
                    column: column_left,
                    keys: keys_left,
                },
                Expr::MapAccess {
                    column: column_right,
                    keys: keys_right,
                },
            ) => ExprEq(column_left) == ExprEq(column_right) && ExprEq(keys_left) == ExprEq(keys_right),
            (Expr::Function(function_left), Expr::Function(function_right)) => todo!(),
            (
                Expr::Case {
                    operand: operand_left,
                    conditions: conditions_left,
                    results: results_left,
                    else_result: else_result_left,
                },
                Expr::Case {
                    operand: operand_right,
                    conditions: conditions_right,
                    results: results_right,
                    else_result: else_result_right,
                },
            ) => todo!(),
            (
                Expr::Exists {
                    subquery: subquery_left,
                    negated: negated_left,
                },
                Expr::Exists {
                    subquery: subquery_right,
                    negated: negated_right,
                },
            ) => todo!(),
            (Expr::Subquery(query_left), Expr::Subquery(query_right)) => todo!(),
            (Expr::GroupingSets(items_left), Expr::GroupingSets(items_right)) => todo!(),
            (Expr::Cube(items_left), Expr::Cube(items_right)) => todo!(),
            (Expr::Rollup(items_left), Expr::Rollup(items_right)) => todo!(),
            (Expr::Tuple(exprs_left), Expr::Tuple(exprs_right)) => todo!(),
            (
                Expr::Struct {
                    values: values_left,
                    fields: fields_left,
                },
                Expr::Struct {
                    values: values_right,
                    fields: fields_right,
                },
            ) => todo!(),
            (
                Expr::Named {
                    expr: expr_left,
                    name: name_left,
                },
                Expr::Named {
                    expr: expr_right,
                    name: name_right,
                },
            ) => todo!(),
            (
                Expr::Dictionary(dictionary_fields_left),
                Expr::Dictionary(dictionary_fields_right),
            ) => todo!(),
            (Expr::Map(map_left), Expr::Map(map_right)) => todo!(),
            _ => false,
        }
    }
}