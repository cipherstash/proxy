use std::{mem, ops::Deref};

use sqlparser::ast::{
    AfterMatchSkip, ConnectBy, Cte, Distinct, ExceptSelectItem, ExcludeSelectItem, Expr, ExprWithAlias, Fetch, FormatClause, FunctionArg, FunctionArgExpr, GroupByExpr, Ident, IdentWithAlias, Interpolate, InterpolateExpr, Join, JoinConstraint, JoinOperator, JsonPath, JsonTableColumn, JsonTableNamedColumn, JsonTableNestedColumn, LateralView, LockClause, MatchRecognizePattern, MatchRecognizeSymbol, Measure, NamedWindowDefinition, NamedWindowExpr, ObjectName, Offset, OrderBy, OrderByExpr, PivotValueSource, Query, RenameSelectItem, ReplaceSelectItem, RowsPerMatch, Select, SelectInto, SelectItem, SetExpr, Setting, SymbolDefinition, TableAlias, TableFactor, TableFunctionArgs, TableWithJoins, Top, TopQuantity, Values, WildcardAdditionalOptions, WindowFrame, WindowFrameBound, WindowSpec, With, WithFill
};

use crate::SqlIdent;

/// Trait for comparing AST nodes for equality that takes into account SQL identifier case comparison rules and
/// superfluous nesting of expressions in parens, e.g. `(foo)` versus `foo`.
///
/// Otherwise it works exactly like [`Eq`].
///
/// # Why is this required?
///
/// 1. To robustly compare expressions in projections and `GROUP BY` clauses to accurately deternine which projetion
/// columns should be aggregated.
///
/// 2. To bury all of the `SqlIdent` logic (future work).
pub(crate) trait SyntacticEq<Rhs = Self> {
    fn syntactic_eq(&self, other: &Rhs) -> bool;
}

impl SyntacticEq for Ident {
    fn syntactic_eq(&self, other: &Self) -> bool {
        SqlIdent(self) == SqlIdent(other)
    }
}

impl SyntacticEq for ObjectName {
    fn syntactic_eq(&self, other: &Self) -> bool {
        self.0.syntactic_eq(&other.0)
    }
}

impl<T: SyntacticEq> SyntacticEq for Vec<T> {
    fn syntactic_eq(&self, other: &Self) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .zip(other.iter())
                .all(|(l, r)| l.syntactic_eq(r))
    }
}

impl<T: SyntacticEq> SyntacticEq for Option<T> {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Some(l), Some(r)) => l.syntactic_eq(r),
            (None, None) => true,
            _ => false,
        }
    }
}

impl<T: SyntacticEq, Rhs: Deref<Target = T>> SyntacticEq<Rhs> for Box<T> {
    fn syntactic_eq(&self, other: &Rhs) -> bool {
        self.as_ref().syntactic_eq(other)
    }
}

impl SyntacticEq for JsonPath {
    fn syntactic_eq(&self, other: &Self) -> bool {
        self.path == other.path
    }
}

impl SyntacticEq for Query {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Query {
            body: body_left,
            order_by: order_by_left,
            limit: limit_left,
            offset: offset_left,
            fetch: fetch_left,
            with: with_left,
            limit_by: limit_by_left,
            locks: locks_left,
            for_clause: for_clause_left,
            settings: settings_left,
            format_clause: format_clause_left,
        } = self;

        let Query {
            body: body_right,
            order_by: order_by_right,
            limit: limit_right,
            offset: offset_right,
            fetch: fetch_right,
            with: with_right,
            limit_by: limit_by_right,
            locks: locks_right,
            for_clause: for_clause_right,
            settings: settings_right,
            format_clause: format_clause_right,
        } = other;

        body_left.syntactic_eq(body_right)
            && order_by_left.syntactic_eq(order_by_right)
            && limit_left.syntactic_eq(limit_right)
            && offset_left.syntactic_eq(offset_right)
            && fetch_left.syntactic_eq(fetch_right)
            && with_left.syntactic_eq(with_right)
            && limit_by_left.syntactic_eq(limit_by_right)
            && locks_left.syntactic_eq(locks_right)
            && for_clause_left == for_clause_right
            && settings_left.syntactic_eq(settings_right)
            && format_clause_left.syntactic_eq(format_clause_right)
    }
}

impl SyntacticEq for FormatClause {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (FormatClause::Identifier(lhs), FormatClause::Identifier(rhs)) => rhs.syntactic_eq(lhs),
            (FormatClause::Null, FormatClause::Null) => true,
            _ => false,
        }
    }
}

impl SyntacticEq for Setting {
    fn syntactic_eq(&self, other: &Self) -> bool {
        self.key.syntactic_eq(&other.key) && self.value == other.value
    }
}

impl SyntacticEq for LockClause {
    fn syntactic_eq(&self, other: &Self) -> bool {
        self.lock_type == other.lock_type
            && self.of.syntactic_eq(&other.of)
            && self.nonblock == other.nonblock
    }
}

impl SyntacticEq for With {
    fn syntactic_eq(&self, other: &Self) -> bool {
        self.recursive == other.recursive && self.cte_tables.syntactic_eq(&other.cte_tables)
    }
}

impl SyntacticEq for Cte {
    fn syntactic_eq(&self, other: &Self) -> bool {
        self.alias.syntactic_eq(&other.alias)
            && self.query.syntactic_eq(&other.query)
            && self.from.syntactic_eq(&other.from)
            && self.materialized == other.materialized
    }
}

impl SyntacticEq for TableAlias {
    fn syntactic_eq(&self, other: &Self) -> bool {
        self.name.syntactic_eq(&other.name) && self.columns.syntactic_eq(&other.columns)
    }
}

impl SyntacticEq for Fetch {
    fn syntactic_eq(&self, other: &Self) -> bool {
        self.with_ties == other.with_ties
            && self.percent == other.percent
            && self.quantity.syntactic_eq(&other.quantity)
    }
}

impl SyntacticEq for OrderBy {
    fn syntactic_eq(&self, other: &Self) -> bool {
        self.exprs.syntactic_eq(&other.exprs) && self.interpolate.syntactic_eq(&other.interpolate)
    }
}

impl SyntacticEq for OrderByExpr {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            expr: expr_lhs,
            nulls_first: nulls_first_lhs,
            with_fill: with_fill_lhs,
            asc: asc_lhs,
        } = self;
        let Self {
            expr: expr_rhs,
            nulls_first: nulls_first_rhs,
            with_fill: with_fill_rhs,
            asc: asc_rhs,
        } = other;

        expr_lhs.syntactic_eq(expr_rhs)
            && nulls_first_lhs == nulls_first_rhs
            && with_fill_lhs.syntactic_eq(with_fill_rhs)
            && asc_lhs == asc_rhs
    }
}

impl SyntacticEq for WithFill {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            from: from_lhs,
            to: to_lhs,
            step: step_lhs,
        } = self;
        let Self {
            from: from_rhs,
            to: to_rhs,
            step: step_rhs,
        } = other;

        from_lhs.syntactic_eq(from_rhs)
            && to_lhs.syntactic_eq(to_rhs)
            && step_lhs.syntactic_eq(step_rhs)
    }
}

impl SyntacticEq for Interpolate {
    fn syntactic_eq(&self, other: &Self) -> bool {
        self.exprs.syntactic_eq(&other.exprs)
    }
}

impl SyntacticEq for InterpolateExpr {
    fn syntactic_eq(&self, other: &Self) -> bool {
        self.column.syntactic_eq(&other.column) && self.expr.syntactic_eq(&other.expr)
    }
}

impl SyntacticEq for Offset {
    fn syntactic_eq(&self, other: &Self) -> bool {
        self.value.syntactic_eq(&other.value) && self.rows == other.rows
    }
}

impl SyntacticEq for SetExpr {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (SetExpr::Select(select_left), SetExpr::Select(select_right)) => {
                select_left.syntactic_eq(select_right)
            }
            (SetExpr::Query(query_left), SetExpr::Query(query_right)) => {
                query_left.syntactic_eq(query_right)
            }
            (
                SetExpr::SetOperation {
                    op: op_left,
                    left: left_left,
                    right: right_left,
                    set_quantifier: set_quantifier_left,
                },
                SetExpr::SetOperation {
                    op: op_right,
                    left: left_right,
                    right: right_right,
                    set_quantifier: set_quantifier_right,
                },
            ) => {
                op_left == op_right
                    && op_left == op_right
                    && left_left.syntactic_eq(left_right)
                    && right_left.syntactic_eq(right_right)
                    && set_quantifier_left == set_quantifier_right
            }
            (SetExpr::Values(values_left), SetExpr::Values(values_right)) => {
                values_left.syntactic_eq(values_right)
            }
            _ => false,
        }
    }
}

impl SyntacticEq for Values {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            explicit_row: explicit_row_lhs,
            rows: rows_lhs,
        } = self;
        let Self {
            explicit_row: explicit_row_rhs,
            rows: rows_rhs,
        } = other;

        *explicit_row_lhs == *explicit_row_rhs && rows_lhs.syntactic_eq(rows_rhs)
    }
}

impl SyntacticEq for Select {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Select {
            distinct: distinct_left,
            top: top_left,
            projection: projection_left,
            from: from_left,
            selection: selection_left,
            group_by: group_by_left,
            having: having_left,
            lateral_views: lateral_views_left,
            top_before_distinct: top_before_distinct_left,
            into: into_left,
            prewhere: prewhere_left,
            cluster_by: cluster_by_left,
            distribute_by: distribute_by_left,
            sort_by: sort_by_left,
            named_window: named_window_left,
            qualify: qualify_left,
            window_before_qualify: window_before_qualify_left,
            value_table_mode: value_table_mode_left,
            connect_by: connect_by_left,
        } = self;

        let Select {
            distinct: distinct_right,
            top: top_right,
            projection: projection_right,
            from: from_right,
            selection: selection_right,
            group_by: group_by_right,
            having: having_right,
            lateral_views: lateral_views_right,
            top_before_distinct: top_before_distinct_right,
            into: into_right,
            prewhere: prewhere_right,
            cluster_by: cluster_by_right,
            distribute_by: distribute_by_right,
            sort_by: sort_by_right,
            named_window: named_window_right,
            qualify: qualify_right,
            window_before_qualify: window_before_qualify_right,
            value_table_mode: value_table_mode_right,
            connect_by: connect_by_right,
        } = other;

        distinct_left.syntactic_eq(distinct_right)
            && top_left.syntactic_eq(top_right)
            && projection_left.syntactic_eq(projection_right)
            && from_left.syntactic_eq(from_right)
            && selection_left.syntactic_eq(selection_right)
            && group_by_left.syntactic_eq(group_by_right)
            && having_left.syntactic_eq(having_right)
            && lateral_views_left.syntactic_eq(lateral_views_right)
            && top_before_distinct_left == top_before_distinct_right
            && into_left.syntactic_eq(into_right)
            && prewhere_left.syntactic_eq(prewhere_right)
            && cluster_by_left.syntactic_eq(cluster_by_right)
            && distribute_by_left.syntactic_eq(distribute_by_right)
            && sort_by_left.syntactic_eq(sort_by_right)
            && named_window_left.syntactic_eq(named_window_right)
            && qualify_left.syntactic_eq(qualify_right)
            && window_before_qualify_left == window_before_qualify_right
            && value_table_mode_left == value_table_mode_right
            && connect_by_left.syntactic_eq(connect_by_right)
    }
}

impl SyntacticEq for ConnectBy {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            condition: condition_lhs,
            relationships: relationships_lhs,
        } = self;
        let Self {
            condition: condition_rhs,
            relationships: relationships_rhs,
        } = other;

        condition_lhs.syntactic_eq(condition_rhs)
            && relationships_lhs.syntactic_eq(relationships_rhs)
    }
}

impl SyntacticEq for NamedWindowDefinition {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self(ident_lhs, named_window_expr_lhs) = self;
        let Self(ident_rhs, named_window_expr_rhs) = other;

        ident_lhs.syntactic_eq(ident_rhs)
            && named_window_expr_lhs.syntactic_eq(named_window_expr_rhs)
    }
}

impl SyntacticEq for NamedWindowExpr {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (NamedWindowExpr::NamedWindow(ident_lhs), NamedWindowExpr::NamedWindow(ident_rhs)) => {
                ident_lhs.syntactic_eq(ident_rhs)
            }
            (
                NamedWindowExpr::WindowSpec(window_spec_lhs),
                NamedWindowExpr::WindowSpec(window_spec_rhs),
            ) => window_spec_lhs.syntactic_eq(window_spec_rhs),
            _ => false,
        }
    }
}

impl SyntacticEq for WindowSpec {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            window_name: window_name_lhs,
            partition_by: partition_by_lhs,
            order_by: order_by_lhs,
            window_frame: window_frame_lhs,
        } = self;
        let Self {
            window_name: window_name_rhs,
            partition_by: partition_by_rhs,
            order_by: order_by_rhs,
            window_frame: window_frame_rhs,
        } = other;

        window_name_lhs.syntactic_eq(window_name_rhs)
            && partition_by_lhs.syntactic_eq(partition_by_rhs)
            && order_by_lhs.syntactic_eq(order_by_rhs)
            && window_frame_lhs.syntactic_eq(window_frame_rhs)
    }
}

impl SyntacticEq for WindowFrame {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            units: units_lhs,
            start_bound: start_bound_lhs,
            end_bound: end_bound_lhs,
        } = self;
        let Self {
            units: units_rhs,
            start_bound: start_bound_rhs,
            end_bound: end_bound_rhs,
        } = other;

        units_lhs == units_rhs
            && start_bound_lhs.syntactic_eq(start_bound_rhs)
            && end_bound_lhs.syntactic_eq(end_bound_rhs)
    }
}

impl SyntacticEq for WindowFrameBound {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (WindowFrameBound::CurrentRow, WindowFrameBound::CurrentRow) => true,
            (WindowFrameBound::Preceding(expr_lhs), WindowFrameBound::Preceding(expr_rhs)) => {
                expr_lhs.syntactic_eq(expr_rhs)
            }
            (WindowFrameBound::Following(expr_lhs), WindowFrameBound::Following(expr_rhs)) => {
                expr_lhs.syntactic_eq(expr_rhs)
            }
            _ => false,
        }
    }
}

impl SyntacticEq for SelectInto {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            temporary: temporary_lhs,
            unlogged: unlogged_lhs,
            table: table_lhs,
            name: name_lhs,
        } = self;
        let Self {
            temporary: temporary_rhs,
            unlogged: unlogged_rhs,
            table: table_rhs,
            name: name_rhs,
        } = other;

        name_lhs.syntactic_eq(name_rhs)
            && *temporary_lhs == *temporary_rhs
            && *unlogged_lhs == *unlogged_rhs
            && *table_lhs == *table_rhs
    }
}

impl SyntacticEq for LateralView {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            lateral_view: lateral_view_lhs,
            lateral_view_name: lateral_view_name_lhs,
            lateral_col_alias: lateral_col_alias_lhs,
            outer: outer_lhs,
        } = self;
        let Self {
            lateral_view: lateral_view_rhs,
            lateral_view_name: lateral_view_name_rhs,
            lateral_col_alias: lateral_col_alias_rhs,
            outer: outer_rhs,
        } = other;

        lateral_view_lhs.syntactic_eq(lateral_view_rhs)
            && lateral_view_name_lhs.syntactic_eq(lateral_view_name_rhs)
            && lateral_col_alias_lhs.syntactic_eq(lateral_col_alias_rhs)
            && *outer_lhs == *outer_rhs
    }
}

impl SyntacticEq for GroupByExpr {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                GroupByExpr::All(group_by_with_modifiers_lhs),
                GroupByExpr::All(group_by_with_modifiers_rhs),
            ) => group_by_with_modifiers_lhs == group_by_with_modifiers_rhs,
            (
                GroupByExpr::Expressions(exprs_lhs, group_by_with_modifiers_lhs),
                GroupByExpr::Expressions(exprs_rhs, group_by_with_modifiers_rhs),
            ) => {
                exprs_lhs.syntactic_eq(exprs_rhs)
                    && group_by_with_modifiers_lhs == group_by_with_modifiers_rhs
            }
            _ => false,
        }
    }
}

impl SyntacticEq for TableWithJoins {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            relation: relation_lhs,
            joins: joins_lhs,
        } = self;
        let Self {
            relation: relation_rhs,
            joins: joins_rhs,
        } = other;

        relation_lhs.syntactic_eq(relation_rhs) && joins_lhs.syntactic_eq(joins_rhs)
    }
}

impl SyntacticEq for Join {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            relation: relation_lhs,
            global: global_lhs,
            join_operator: join_operator_lhs,
        } = self;
        let Self {
            relation: relation_rhs,
            global: global_rhs,
            join_operator: join_operator_rhs,
        } = other;

        relation_lhs.syntactic_eq(relation_rhs)
            && *global_lhs == *global_rhs
            && join_operator_lhs.syntactic_eq(join_operator_rhs)
    }
}

impl SyntacticEq for JoinOperator {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                JoinOperator::Inner(join_constraint_lhs),
                JoinOperator::Inner(join_constraint_rhs),
            )
            | (
                JoinOperator::LeftOuter(join_constraint_lhs),
                JoinOperator::LeftOuter(join_constraint_rhs),
            )
            | (
                JoinOperator::RightOuter(join_constraint_lhs),
                JoinOperator::RightOuter(join_constraint_rhs),
            )
            | (
                JoinOperator::FullOuter(join_constraint_lhs),
                JoinOperator::FullOuter(join_constraint_rhs),
            )
            | (
                JoinOperator::LeftSemi(join_constraint_lhs),
                JoinOperator::LeftSemi(join_constraint_rhs),
            )
            | (
                JoinOperator::RightSemi(join_constraint_lhs),
                JoinOperator::RightSemi(join_constraint_rhs),
            )
            | (
                JoinOperator::LeftAnti(join_constraint_lhs),
                JoinOperator::LeftAnti(join_constraint_rhs),
            )
            | (
                JoinOperator::RightAnti(join_constraint_lhs),
                JoinOperator::RightAnti(join_constraint_rhs),
            ) => join_constraint_lhs.syntactic_eq(join_constraint_rhs),
            (JoinOperator::CrossJoin, JoinOperator::CrossJoin)
            | (JoinOperator::CrossApply, JoinOperator::CrossApply)
            | (JoinOperator::OuterApply, JoinOperator::OuterApply) => true,
            (
                JoinOperator::AsOf {
                    match_condition: match_condition_lhs,
                    constraint: constraint_lhs,
                },
                JoinOperator::AsOf {
                    match_condition: match_condition_rhs,
                    constraint: constraint_rhs,
                },
            ) => {
                match_condition_lhs.syntactic_eq(match_condition_rhs)
                    && constraint_lhs.syntactic_eq(constraint_rhs)
            }
            _ => false,
        }
    }
}

impl SyntacticEq for JoinConstraint {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (JoinConstraint::On(expr_lhs), JoinConstraint::On(expr_rhs)) => {
                expr_lhs.syntactic_eq(expr_rhs)
            }
            (JoinConstraint::Using(idents_lhs), JoinConstraint::Using(idents_rhs)) => {
                idents_lhs.syntactic_eq(idents_rhs)
            }
            (JoinConstraint::Natural, JoinConstraint::Natural) => todo!(),
            (JoinConstraint::None, JoinConstraint::None) => todo!(),
            _ => false,
        }
    }
}

impl SyntacticEq for TableFunctionArgs {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            args: args_lhs,
            settings: settings_lhs,
        } = self;
        let Self {
            args: args_rhs,
            settings: settings_rhs,
        } = other;

        args_lhs.syntactic_eq(args_rhs) && settings_lhs.syntactic_eq(settings_rhs)
    }
}

impl SyntacticEq for FunctionArg {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                FunctionArg::Named {
                    name: name_lhs,
                    arg: arg_lhs,
                    operator: operator_lhs,
                },
                FunctionArg::Named {
                    name: name_rhs,
                    arg: arg_rhs,
                    operator: operator_rhs,
                },
            ) => {
                name_lhs.syntactic_eq(name_rhs)
                    && arg_lhs.syntactic_eq(arg_rhs)
                    && operator_lhs == operator_rhs
            }
            (
                FunctionArg::Unnamed(function_arg_expr_lhs),
                FunctionArg::Unnamed(function_arg_expr_rhs),
            ) => function_arg_expr_lhs.syntactic_eq(function_arg_expr_rhs),
            _ => false,
        }
    }
}

impl SyntacticEq for FunctionArgExpr {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (FunctionArgExpr::Expr(expr_lhs), FunctionArgExpr::Expr(expr_rhs)) => {
                expr_lhs.syntactic_eq(expr_rhs)
            }
            (
                FunctionArgExpr::QualifiedWildcard(object_name_lhs),
                FunctionArgExpr::QualifiedWildcard(object_name_rhs),
            ) => object_name_lhs.syntactic_eq(object_name_rhs),
            (FunctionArgExpr::Wildcard, FunctionArgExpr::Wildcard) => true,
            _ => false,
        }
    }
}

impl SyntacticEq for TableFactor {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                TableFactor::Table {
                    name: name_lhs,
                    alias: alias_lhs,
                    args: args_lhs,
                    with_hints: with_hints_lhs,
                    version: version_lhs,
                    with_ordinality: with_ordinality_lhs,
                    partitions: partitions_lhs,
                },
                TableFactor::Table {
                    name: name_rhs,
                    alias: alias_rhs,
                    args: args_rhs,
                    with_hints: with_hints_rhs,
                    version: version_rhs,
                    with_ordinality: with_ordinality_rhs,
                    partitions: partitions_rhs,
                },
            ) => {
                name_lhs.syntactic_eq(name_rhs)
                    && alias_lhs.syntactic_eq(alias_lhs)
                    && args_lhs.syntactic_eq(args_rhs)
                    && with_hints_lhs.syntactic_eq(with_hints_rhs)
                    && version_lhs == version_rhs
                    && with_ordinality_lhs == with_ordinality_rhs
                    && partitions_lhs.syntactic_eq(partitions_rhs)
            }
            (
                TableFactor::Derived {
                    lateral: lateral_lhs,
                    subquery: subquery_lhs,
                    alias: alias_lhs,
                },
                TableFactor::Derived {
                    lateral: lateral_rhs,
                    subquery: subquery_rhs,
                    alias: alias_rhs,
                },
            ) => {
                lateral_lhs == lateral_rhs
                    && subquery_lhs.syntactic_eq(subquery_rhs)
                    && alias_lhs.syntactic_eq(alias_rhs)
            }
            (
                TableFactor::TableFunction {
                    expr: expr_lhs,
                    alias: alias_lhs,
                },
                TableFactor::TableFunction {
                    expr: expr_rhs,
                    alias: alias_rhs,
                },
            ) => expr_lhs.syntactic_eq(expr_rhs) && alias_lhs.syntactic_eq(alias_rhs),
            (
                TableFactor::Function {
                    lateral: lateral_lhs,
                    name: name_lhs,
                    args: args_lhs,
                    alias: alias_lhs,
                },
                TableFactor::Function {
                    lateral: lateral_rhs,
                    name: name_rhs,
                    args: args_rhs,
                    alias: alias_rhs,
                },
            ) => {
                *lateral_lhs == *lateral_rhs
                    && name_rhs.syntactic_eq(name_lhs)
                    && args_rhs.syntactic_eq(args_lhs)
                    && alias_lhs.syntactic_eq(alias_rhs)
            }
            (
                TableFactor::UNNEST {
                    alias: alias_lhs,
                    array_exprs: array_exprs_lhs,
                    with_offset: with_offset_lhs,
                    with_offset_alias: with_offset_alias_lhs,
                    with_ordinality: with_ordinality_lhs,
                },
                TableFactor::UNNEST {
                    alias: alias_rhs,
                    array_exprs: array_exprs_rhs,
                    with_offset: with_offset_rhs,
                    with_offset_alias: with_offset_alias_rhs,
                    with_ordinality: with_ordinality_rhs,
                },
            ) => {
                alias_lhs.syntactic_eq(alias_rhs)
                    && array_exprs_lhs.syntactic_eq(array_exprs_rhs)
                    && with_offset_lhs == with_offset_rhs
                    && with_offset_alias_lhs.syntactic_eq(with_offset_alias_rhs)
                    && with_ordinality_lhs == with_ordinality_rhs
            }
            (
                TableFactor::JsonTable {
                    json_expr: json_expr_lhs,
                    json_path: json_path_lhs,
                    columns: columns_lhs,
                    alias: alias_lhs,
                },
                TableFactor::JsonTable {
                    json_expr: json_expr_rhs,
                    json_path: json_path_rhs,
                    columns: columns_rhs,
                    alias: alias_rhs,
                },
            ) => {
                json_expr_lhs.syntactic_eq(json_expr_rhs)
                    && json_path_lhs == json_path_rhs
                    && columns_lhs.syntactic_eq(columns_rhs)
                    && alias_lhs.syntactic_eq(alias_rhs)
            }
            (
                TableFactor::NestedJoin {
                    table_with_joins: table_with_joins_lhs,
                    alias: alias_lhs,
                },
                TableFactor::NestedJoin {
                    table_with_joins: table_with_joins_rhs,
                    alias: alias_rhs,
                },
            ) => {
                table_with_joins_lhs.syntactic_eq(table_with_joins_rhs)
                    && alias_lhs.syntactic_eq(alias_rhs)
            }
            (
                TableFactor::Pivot {
                    table: table_lhs,
                    aggregate_functions: aggregate_functions_lhs,
                    value_column: value_column_lhs,
                    value_source: value_source_lhs,
                    default_on_null: default_on_null_lhs,
                    alias: alias_lhs,
                },
                TableFactor::Pivot {
                    table: table_rhs,
                    aggregate_functions: aggregate_functions_rhs,
                    value_column: value_column_rhs,
                    value_source: value_source_rhs,
                    default_on_null: default_on_null_rhs,
                    alias: alias_rhs,
                },
            ) => {
                table_lhs.syntactic_eq(table_rhs)
                    && aggregate_functions_lhs.syntactic_eq(aggregate_functions_rhs)
                    && value_column_lhs.syntactic_eq(value_column_rhs)
                    && value_source_lhs.syntactic_eq(value_source_rhs)
                    && default_on_null_lhs.syntactic_eq(default_on_null_rhs)
                    && alias_lhs.syntactic_eq(alias_rhs)
            }
            (
                TableFactor::Unpivot {
                    table: table_lhs,
                    value: value_lhs,
                    name: name_lhs,
                    columns: columns_lhs,
                    alias: alias_lhs,
                },
                TableFactor::Unpivot {
                    table: table_rhs,
                    value: value_rhs,
                    name: name_rhs,
                    columns: columns_rhs,
                    alias: alias_rhs,
                },
            ) => {
                table_lhs.syntactic_eq(table_rhs)
                    && value_lhs.syntactic_eq(value_rhs)
                    && name_lhs.syntactic_eq(name_rhs)
                    && columns_lhs.syntactic_eq(columns_rhs)
                    && alias_lhs.syntactic_eq(alias_rhs)
            }
            (
                TableFactor::MatchRecognize {
                    table: table_lhs,
                    partition_by: partition_by_lhs,
                    order_by: order_by_lhs,
                    measures: measures_lhs,
                    rows_per_match: rows_per_match_lhs,
                    after_match_skip: after_match_skip_lhs,
                    pattern: pattern_lhs,
                    symbols: symbols_lhs,
                    alias: alias_lhs,
                },
                TableFactor::MatchRecognize {
                    table: table_rhs,
                    partition_by: partition_by_rhs,
                    order_by: order_by_rhs,
                    measures: measures_rhs,
                    rows_per_match: rows_per_match_rhs,
                    after_match_skip: after_match_skip_rhs,
                    pattern: pattern_rhs,
                    symbols: symbols_rhs,
                    alias: alias_rhs,
                },
            ) => {
                table_lhs.syntactic_eq(table_rhs)
                    && partition_by_lhs.syntactic_eq(partition_by_rhs)
                    && order_by_lhs.syntactic_eq(order_by_rhs)
                    && measures_lhs.syntactic_eq(measures_rhs)
                    && rows_per_match_lhs == rows_per_match_rhs
                    && after_match_skip_lhs.syntactic_eq(after_match_skip_rhs)
                    && pattern_lhs.syntactic_eq(pattern_rhs)
                    && symbols_lhs.syntactic_eq(symbols_rhs)
                    && alias_lhs.syntactic_eq(alias_rhs)
            }
            _ => false,
        }
    }
}

impl SyntacticEq for SymbolDefinition {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            symbol: symbol_lhs,
            definition: defininition_lhs,
        } = self;
        let Self {
            symbol: symbol_rhs,
            definition: defininition_rhs,
        } = other;

        symbol_lhs.syntactic_eq(symbol_rhs) && defininition_lhs.syntactic_eq(defininition_rhs)
    }
}

impl SyntacticEq for JsonTableNamedColumn {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            name: name_lhs,
            r#type: r#type_lhs,
            path: path_lhs,
            exists: exists_lhs,
            on_empty: on_empty_lhs,
            on_error: on_error_lhs,
        } = self;
        let Self {
            name: name_rhs,
            r#type: r#type_rhs,
            path: path_rhs,
            exists: exists_rhs,
            on_empty: on_empty_rhs,
            on_error: on_error_rhs,
        } = other;

        name_lhs.syntactic_eq(name_rhs)
            && r#type_lhs == r#type_rhs
            && path_lhs == path_rhs
            && *exists_lhs == *exists_rhs
            && on_empty_lhs == on_empty_rhs
            && on_error_lhs == on_error_rhs
    }
}

impl SyntacticEq for MatchRecognizePattern {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                MatchRecognizePattern::Symbol(match_recognize_symbol_lhs),
                MatchRecognizePattern::Symbol(match_recognize_symbol_rhs),
            )
            | (
                MatchRecognizePattern::Exclude(match_recognize_symbol_lhs),
                MatchRecognizePattern::Exclude(match_recognize_symbol_rhs),
            ) => match_recognize_symbol_lhs.syntactic_eq(match_recognize_symbol_rhs),
            (
                MatchRecognizePattern::Permute(match_recognize_symbols_lhs),
                MatchRecognizePattern::Permute(match_recognize_symbols_rhs),
            ) => match_recognize_symbols_lhs.syntactic_eq(match_recognize_symbols_rhs),
            (
                MatchRecognizePattern::Concat(match_recognize_patterns_lhs),
                MatchRecognizePattern::Concat(match_recognize_patterns_rhs),
            ) => match_recognize_patterns_rhs.syntactic_eq(match_recognize_patterns_rhs),
            (
                MatchRecognizePattern::Group(match_recognize_pattern_lhs),
                MatchRecognizePattern::Group(match_recognize_pattern_rhs),
            ) => match_recognize_pattern_lhs.syntactic_eq(match_recognize_pattern_rhs),
            (
                MatchRecognizePattern::Alternation(match_recognize_patterns_lhs),
                MatchRecognizePattern::Alternation(match_recognize_patterns_rhs),
            ) => match_recognize_patterns_lhs.syntactic_eq(match_recognize_patterns_rhs),
            (
                MatchRecognizePattern::Repetition(
                    match_recognize_pattern_lhs,
                    repetition_quantifier_lhs,
                ),
                MatchRecognizePattern::Repetition(
                    match_recognize_pattern_rhs,
                    repetition_quantifier_rhs,
                ),
            ) => {
                match_recognize_pattern_lhs.syntactic_eq(match_recognize_pattern_rhs)
                    && repetition_quantifier_lhs == repetition_quantifier_rhs
            }
            _ => false,
        }
    }
}

impl SyntacticEq for MatchRecognizeSymbol {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (MatchRecognizeSymbol::Named(ident_lhs), MatchRecognizeSymbol::Named(ident_rhs)) => {
                ident_lhs.syntactic_eq(ident_rhs)
            }
            (MatchRecognizeSymbol::Start, MatchRecognizeSymbol::Start)
            | (MatchRecognizeSymbol::End, MatchRecognizeSymbol::End) => true,
            _ => false,
        }
    }
}

impl SyntacticEq for AfterMatchSkip {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (AfterMatchSkip::PastLastRow, AfterMatchSkip::PastLastRow)
            | (AfterMatchSkip::ToNextRow, AfterMatchSkip::ToNextRow) => true,
            (AfterMatchSkip::ToFirst(ident_lhs), AfterMatchSkip::ToFirst(ident_rhs))
            | (AfterMatchSkip::ToLast(ident_lhs), AfterMatchSkip::ToLast(ident_rhs)) => {
                ident_lhs.syntactic_eq(ident_rhs)
            }
            _ => false,
        }
    }
}

impl SyntacticEq for Measure {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            expr: expr_lhs,
            alias: alias_lhs,
        } = self;
        let Self {
            expr: expr_rhs,
            alias: alias_rhs,
        } = other;

        expr_lhs.syntactic_eq(expr_rhs) && alias_lhs.syntactic_eq(alias_rhs)
    }
}

impl SyntacticEq for PivotValueSource {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (PivotValueSource::List(items_lhs), PivotValueSource::List(items_rhs)) => {
                items_lhs.syntactic_eq(items_rhs)
            }
            (
                PivotValueSource::Any(order_by_exprs_lhs),
                PivotValueSource::Any(order_by_exprs_rhs),
            ) => order_by_exprs_lhs.syntactic_eq(order_by_exprs_rhs),
            (PivotValueSource::Subquery(query_lhs), PivotValueSource::Subquery(query_rhs)) => {
                query_lhs.syntactic_eq(query_rhs)
            }
            _ => false,
        }
    }
}

impl SyntacticEq for ExprWithAlias {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            expr: expr_lhs,
            alias: alias_lhs,
        } = self;
        let Self {
            expr: expr_rhs,
            alias: alias_rhs,
        } = other;

        expr_lhs.syntactic_eq(expr_rhs) && alias_lhs.syntactic_eq(alias_rhs)
    }
}

impl SyntacticEq for JsonTableColumn {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                JsonTableColumn::Named(json_table_named_column_lhs),
                JsonTableColumn::Named(json_table_named_column_rhs),
            ) => json_table_named_column_lhs.syntactic_eq(json_table_named_column_rhs),
            (
                JsonTableColumn::ForOrdinality(ident_lhs),
                JsonTableColumn::ForOrdinality(ident_rhs),
            ) => ident_lhs.syntactic_eq(ident_rhs),
            (
                JsonTableColumn::Nested(json_table_nested_column_lhs),
                JsonTableColumn::Nested(json_table_nested_column_rhs),
            ) => json_table_nested_column_lhs.syntactic_eq(json_table_nested_column_rhs),
            _ => false,
        }
    }
}

impl SyntacticEq for JsonTableNestedColumn {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            path: path_lhs,
            columns: columns_lhs,
        } = self;
        let Self {
            path: path_rhs,
            columns: columns_rhs,
        } = other;

        path_lhs == path_rhs && columns_lhs.syntactic_eq(columns_rhs)
    }
}

impl SyntacticEq for Distinct {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Distinct::Distinct, Distinct::Distinct) => true,
            (Distinct::On(exprs_left), Distinct::On(exprs_right)) => {
                exprs_left.syntactic_eq(exprs_right)
            }
            _ => false,
        }
    }
}

impl SyntacticEq for Top {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            percent: percent_left,
            with_ties: with_ties_left,
            quantity: quantity_left,
        } = self;

        let Self {
            percent: percent_right,
            with_ties: with_ties_right,
            quantity: quantity_right,
        } = other;

        percent_left == percent_right
            && with_ties_left == with_ties_right
            && quantity_left.syntactic_eq(quantity_right)
    }
}

impl SyntacticEq for SelectItem {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (SelectItem::UnnamedExpr(expr_lhs), SelectItem::UnnamedExpr(expr_rhs)) => {
                expr_lhs.syntactic_eq(expr_rhs)
            }
            (
                SelectItem::ExprWithAlias {
                    expr: expr_lhs,
                    alias: alias_lhs,
                },
                SelectItem::ExprWithAlias {
                    expr: expr_rhs,
                    alias: alias_rhs,
                },
            ) => expr_lhs.syntactic_eq(expr_rhs) && alias_lhs.syntactic_eq(alias_rhs),
            (
                SelectItem::QualifiedWildcard(object_name_lhs, wildcard_additional_options_lhs),
                SelectItem::QualifiedWildcard(object_name_rhs, wildcard_additional_options_rhs),
            ) => {
                object_name_lhs.syntactic_eq(object_name_rhs)
                    && wildcard_additional_options_lhs.syntactic_eq(wildcard_additional_options_rhs)
            }
            (
                SelectItem::Wildcard(wildcard_additional_options_lhs),
                SelectItem::Wildcard(wildcard_additional_options_rhs),
            ) => wildcard_additional_options_lhs.syntactic_eq(wildcard_additional_options_rhs),
            _ => false,
        }
    }
}

impl SyntacticEq for WildcardAdditionalOptions {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            opt_ilike: opt_ilike_lhs,
            opt_exclude: opt_exclude_lhs,
            opt_except: opt_except_lhs,
            opt_replace: opt_replace_lhs,
            opt_rename: opt_rename_lhs,
        } = self;

        let Self {
            opt_ilike: opt_ilike_rhs,
            opt_exclude: opt_exclude_rhs,
            opt_except: opt_except_rhs,
            opt_replace: opt_replace_rhs,
            opt_rename: opt_rename_rhs,
        } = other;

        opt_ilike_lhs == opt_ilike_rhs
            && opt_exclude_lhs.syntactic_eq(opt_exclude_rhs)
            && opt_except_lhs.syntactic_eq(opt_except_rhs)
            && opt_replace_lhs.syntactic_eq(opt_replace_rhs)
            && opt_rename_lhs.syntactic_eq(opt_rename_rhs)
    }
}

impl SyntacticEq for RenameSelectItem {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                RenameSelectItem::Single(ident_with_alias_lhs),
                RenameSelectItem::Single(ident_with_alias_rhs),
            ) => ident_with_alias_lhs.syntactic_eq(ident_with_alias_rhs),
            (RenameSelectItem::Multiple(items_lhs), RenameSelectItem::Multiple(items_rhs)) => {
                items_lhs.syntactic_eq(items_rhs)
            }
            _ => false,
        }
    }
}

impl SyntacticEq for IdentWithAlias {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self { ident: ident_lhs, alias: alias_lhs } = self;
        let Self { ident: ident_rhs, alias: alias_rhs } = other;

        ident_lhs.syntactic_eq(ident_rhs) && alias_lhs.syntactic_eq(alias_rhs)
    }
}

impl SyntacticEq for ExcludeSelectItem {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ExcludeSelectItem::Single(ident_lhs), ExcludeSelectItem::Single(ident_rhs)) => {
                ident_lhs.syntactic_eq(ident_rhs)
            }
            (ExcludeSelectItem::Multiple(idents_lhs), ExcludeSelectItem::Multiple(idents_rhs)) => {
                idents_lhs.syntactic_eq(idents_rhs)
            }
            _ => false,
        }
    }
}

impl SyntacticEq for ExceptSelectItem {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            first_element: first_element_lhs,
            additional_elements: additional_elements_lhs,
        } = self;
        let Self {
            first_element: first_element_rhs,
            additional_elements: additional_elements_rhs,
        } = other;

        first_element_lhs.syntactic_eq(first_element_rhs)
            && additional_elements_lhs.syntactic_eq(additional_elements_rhs)
    }
}

impl SyntacticEq for TopQuantity {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (TopQuantity::Expr(expr_left), TopQuantity::Expr(expr_right)) => {
                expr_left.syntactic_eq(expr_right)
            }
            (TopQuantity::Constant(c_left), TopQuantity::Constant(c_right)) => c_left == c_right,
            _ => false,
        }
    }
}

impl SyntacticEq for ReplaceSelectItem {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self { items: items_lhs } = self;
        let Self { items: items_rhs } = other;

        items_lhs.syntactic_eq(items_rhs.as_slice())
    }
}

impl SyntacticEq for Expr {
    fn syntactic_eq(&self, other: &Self) -> bool {
        // Expr::Nested(_) requires special handling because the parens are superfluous when it comes to equality.
        match (self, other) {
            (Expr::Nested(expr_left), expr_right) => {
                return (&**expr_left).syntactic_eq(expr_right)
            }
            (expr_left, Expr::Nested(expr_right)) => return expr_left.syntactic_eq(expr_right),
            _ => {}
        }

        // If the discriminants are different then the nodes are different, so we can bail out.
        if mem::discriminant(self) != mem::discriminant(other) {
            return false;
        }

        match (self, other) {
            (Expr::Identifier(ident_left), Expr::Identifier(ident_right)) => {
                ident_left.syntactic_eq(ident_right)
            }
            (Expr::CompoundIdentifier(idents_left), Expr::CompoundIdentifier(idents_right)) => {
                idents_left.syntactic_eq(idents_right)
            }
            (
                Expr::JsonAccess {
                    value: value_left,
                    path: path_left,
                },
                Expr::JsonAccess {
                    value: value_right,
                    path: path_right,
                },
            ) => value_left.syntactic_eq(value_right) && path_left.syntactic_eq(path_right),
            (
                Expr::CompositeAccess {
                    expr: expr_left,
                    key: key_left,
                },
                Expr::CompositeAccess {
                    expr: expr_right,
                    key: key_right,
                },
            ) => expr_left.syntactic_eq(expr_right) && key_left.syntactic_eq(key_right),
            (Expr::IsFalse(expr_left), Expr::IsFalse(expr_right)) => {
                expr_left.syntactic_eq(expr_right)
            }
            (Expr::IsNotFalse(expr_left), Expr::IsNotFalse(expr_right)) => {
                expr_left.syntactic_eq(expr_right)
            }
            (Expr::IsTrue(expr_left), Expr::IsTrue(expr_right)) => {
                expr_left.syntactic_eq(expr_right)
            }
            (Expr::IsNotTrue(expr_left), Expr::IsNotTrue(expr_right)) => {
                expr_left.syntactic_eq(expr_right)
            }
            (Expr::IsNull(expr_left), Expr::IsNull(expr_right)) => {
                expr_left.syntactic_eq(expr_right)
            }
            (Expr::IsNotNull(expr_left), Expr::IsNotNull(expr_right)) => {
                expr_left.syntactic_eq(expr_right)
            }
            (Expr::IsUnknown(expr_left), Expr::IsUnknown(expr_right)) => {
                expr_left.syntactic_eq(expr_right)
            }
            (Expr::IsNotUnknown(expr_left), Expr::IsNotUnknown(expr_right)) => {
                expr_left.syntactic_eq(expr_right)
            }
            (
                Expr::IsDistinctFrom(expr_left, expr1_left),
                Expr::IsDistinctFrom(expr_right, expr1_right),
            ) => expr_left.syntactic_eq(expr_right) && expr1_left.syntactic_eq(expr1_right),
            (
                Expr::IsNotDistinctFrom(expr_left, expr1_left),
                Expr::IsNotDistinctFrom(expr_right, expr1_right),
            ) => expr_left.syntactic_eq(expr_right) && expr1_left.syntactic_eq(expr1_right),
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
                expr_left.syntactic_eq(expr_right)
                    && list_left
                        .iter()
                        .zip(list_right.iter())
                        .all(|(l, r)| l.syntactic_eq(r))
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
                expr_left.syntactic_eq(expr_right)
                    && subquery_left.syntactic_eq(subquery_right)
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
                expr_left.syntactic_eq(expr_right)
                    && array_expr_left.syntactic_eq(array_expr_right)
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
                expr_left.syntactic_eq(expr_right)
                    && negated_left == negated_right
                    && low_left.syntactic_eq(low_right)
                    && high_left.syntactic_eq(high_right)
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
                left_left.syntactic_eq(left_right)
                    && right_left.syntactic_eq(right_right)
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
            ) => {
                negated_left == negated_right
                    && any_left == any_right
                    && expr_left.syntactic_eq(expr_right)
                    && pattern_left.syntactic_eq(pattern_right)
                    && escape_char_left == escape_char_right
            }
            (
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
                    && expr_left.syntactic_eq(expr_right)
                    && pattern_left.syntactic_eq(pattern_right)
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
                    && expr_left.syntactic_eq(expr_right)
                    && pattern_left.syntactic_eq(pattern_right)
                    && escape_char_left == escape_char_right
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
                    && expr_left.syntactic_eq(expr_right)
                    && pattern_left.syntactic_eq(pattern_right)
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
                left_left.syntactic_eq(left_right)
                    && compare_op_left == compare_op_right
                    && right_left.syntactic_eq(right_right)
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
                left_left.syntactic_eq(left_right)
                    && compare_op_left == compare_op_right
                    && right_left.syntactic_eq(right_right)
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
            ) => op_left == op_right && expr_left.syntactic_eq(expr_right),
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
                    && expr_left.syntactic_eq(expr_right)
                    && data_type_left == data_type_right
                    && charset_left.syntactic_eq(charset_right)
                    && target_before_value_left == target_before_value_right
                    && styles_left.syntactic_eq(styles_right)
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
                    && expr_left.syntactic_eq(expr_right)
                    && data_type_left.syntactic_eq(data_type_right)
                    && format_left.syntactic_eq(format_right)
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
                timestamp_left.syntactic_eq(timestamp_right)
                    && time_zone_left.syntactic_eq(time_zone_right)
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
                field_left.syntactic_eq(field_right)
                    && syntax_left.syntactic_eq(syntax_right)
                    && expr_left.syntactic_eq(expr_right)
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
            ) => expr_left.syntactic_eq(expr_right) && field_left.syntactic_eq(field_right),
            (
                Expr::Floor {
                    expr: expr_left,
                    field: field_left,
                },
                Expr::Floor {
                    expr: expr_right,
                    field: field_right,
                },
            ) => expr_left.syntactic_eq(expr_right) && field_left.syntactic_eq(field_right),
            (
                Expr::Position {
                    expr: expr_left,
                    r#in: in_left,
                },
                Expr::Position {
                    expr: expr_right,
                    r#in: in_right,
                },
            ) => expr_left.syntactic_eq(expr_right) && in_left.syntactic_eq(in_right),
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
                expr_left.syntactic_eq(expr_right)
                    && substring_from_left.syntactic_eq(substring_from_right)
                    && substring_for_left.syntactic_eq(substring_for_right)
                    && special_left.syntactic_eq(special_right)
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
                expr_left.syntactic_eq(expr_right)
                    && trim_where_left.syntactic_eq(trim_where_right)
                    && trim_what_left.syntactic_eq(trim_what_right)
                    && trim_characters_left.syntactic_eq(trim_characters_right)
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
                expr_left.syntactic_eq(expr_right)
                    && overlay_what_left.syntactic_eq(overlay_what_right)
                    && overlay_from_left.syntactic_eq(overlay_from_right)
                    && overlay_for_left.syntactic_eq(overlay_for_right)
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
            ) => expr_left.syntactic_eq(expr_right) && collation_left.syntactic_eq(collation_right),
            (Expr::Nested(expr_left), Expr::Nested(expr_right)) => {
                expr_left.syntactic_eq(expr_right)
            }
            (Expr::Value(value_left), Expr::Value(value_right)) => {
                value_left.syntactic_eq(value_right)
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
            ) => {
                introducer_left.syntactic_eq(introducer_right)
                    && value_left.syntactic_eq(value_right)
            }
            (
                Expr::TypedString {
                    data_type: data_type_left,
                    value: value_left,
                },
                Expr::TypedString {
                    data_type: data_type_right,
                    value: value_right,
                },
            ) => {
                data_type_left.syntactic_eq(data_type_right) && value_left.syntactic_eq(value_right)
            }
            (
                Expr::MapAccess {
                    column: column_left,
                    keys: keys_left,
                },
                Expr::MapAccess {
                    column: column_right,
                    keys: keys_right,
                },
            ) => column_left.syntactic_eq(column_right) && keys_left.syntactic_eq(keys_right),
            (Expr::Function(function_left), Expr::Function(function_right)) => {
                function_left.syntactic_eq(function_right)
            }
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
            ) => {
                operand_left.syntactic_eq(operand_right)
                    && conditions_left.syntactic_eq(conditions_right)
                    && results_left.syntactic_eq(results_right)
                    && else_result_left.syntactic_eq(else_result_right)
            }
            (
                Expr::Exists {
                    subquery: subquery_left,
                    negated: negated_left,
                },
                Expr::Exists {
                    subquery: subquery_right,
                    negated: negated_right,
                },
            ) => subquery_left.syntactic_eq(subquery_right) && negated_left == negated_right,
            (Expr::Subquery(query_left), Expr::Subquery(query_right)) => {
                query_left.syntactic_eq(query_right)
            }
            (Expr::GroupingSets(items_left), Expr::GroupingSets(items_right)) => {
                items_left.syntactic_eq(items_right)
            }
            (Expr::Cube(items_left), Expr::Cube(items_right)) => {
                items_left.syntactic_eq(items_right)
            }
            (Expr::Rollup(items_left), Expr::Rollup(items_right)) => {
                items_left.syntactic_eq(items_right)
            }
            (Expr::Tuple(exprs_left), Expr::Tuple(exprs_right)) => {
                exprs_left.syntactic_eq(exprs_right)
            }
            (
                Expr::Struct {
                    values: values_left,
                    fields: fields_left,
                },
                Expr::Struct {
                    values: values_right,
                    fields: fields_right,
                },
            ) => values_left.syntactic_eq(values_right) && fields_left.syntactic_eq(fields_right),
            (
                Expr::Named {
                    expr: expr_left,
                    name: name_left,
                },
                Expr::Named {
                    expr: expr_right,
                    name: name_right,
                },
            ) => expr_left.syntactic_eq(expr_right) && name_left.syntactic_eq(name_right),
            (
                Expr::Dictionary(dictionary_fields_left),
                Expr::Dictionary(dictionary_fields_right),
            ) => dictionary_fields_left.syntactic_eq(dictionary_fields_right),
            (Expr::Map(map_left), Expr::Map(map_right)) => map_left.syntactic_eq(map_right),
            (
                Expr::Subscript {
                    expr: expr_left,
                    subscript: subscript_left,
                },
                Expr::Subscript {
                    expr: expr_right,
                    subscript: subscript_right,
                },
            ) => expr_left.syntactic_eq(expr_right) && subscript_left.syntactic_eq(subscript_right),
            (Expr::Array(array_left), Expr::Array(array_right)) => {
                array_left.syntactic_eq(array_right)
            }
            (Expr::Interval(interval_left), Expr::Interval(interval_right)) => {
                interval_left.syntactic_eq(interval_right)
            }
            (
                Expr::MatchAgainst {
                    columns: columns_left,
                    match_value: match_value_left,
                    opt_search_modifier: opt_search_modifier_left,
                },
                Expr::MatchAgainst {
                    columns: columns_right,
                    match_value: match_value_right,
                    opt_search_modifier: opt_search_modifier_right,
                },
            ) => {
                columns_left.syntactic_eq(columns_right)
                    && match_value_left.syntactic_eq(match_value_right)
                    && opt_search_modifier_left.syntactic_eq(opt_search_modifier_right)
            }
            (Expr::Wildcard, Expr::Wildcard) => true,
            (
                Expr::QualifiedWildcard(object_name_left),
                Expr::QualifiedWildcard(object_name_right),
            ) => object_name_left.syntactic_eq(object_name_right),
            (Expr::OuterJoin(expr_left), Expr::OuterJoin(expr_right)) => {
                expr_left.syntactic_eq(expr_right)
            }
            (Expr::Prior(expr_left), Expr::Prior(expr_right)) => expr_left.syntactic_eq(expr_right),
            (Expr::Lambda(lambda_function_left), Expr::Lambda(lambda_function_right)) => {
                lambda_function_left.syntactic_eq(lambda_function_right)
            }
        }
    }
}
