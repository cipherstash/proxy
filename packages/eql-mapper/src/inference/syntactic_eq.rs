use std::mem;

use sqlparser::ast::{
    AfterMatchSkip, Array, CeilFloorKind, ConnectBy, Cte, DateTimeField, DictionaryField, Distinct,
    ExceptSelectItem, ExcludeSelectItem, Expr, ExprWithAlias, Fetch, FormatClause, Function,
    FunctionArg, FunctionArgExpr, FunctionArgumentClause, FunctionArgumentList, FunctionArguments,
    GroupByExpr, HavingBound, Ident, IdentWithAlias, Interpolate, InterpolateExpr, Interval, Join,
    JoinConstraint, JoinOperator, JsonPath, JsonTableColumn, JsonTableNamedColumn,
    JsonTableNestedColumn, LambdaFunction, LateralView, ListAggOnOverflow, LockClause, Map,
    MapAccessKey, MapEntry, MatchRecognizePattern, MatchRecognizeSymbol, Measure,
    NamedWindowDefinition, NamedWindowExpr, ObjectName, Offset, OneOrManyWithParens, OrderBy,
    OrderByExpr, PivotValueSource, Query, RenameSelectItem, ReplaceSelectElement,
    ReplaceSelectItem, Select, SelectInto, SelectItem, SetExpr, Setting, StructField, Subscript,
    SymbolDefinition, TableAlias, TableFactor, TableFunctionArgs, TableWithJoins, Top, TopQuantity,
    Values, WildcardAdditionalOptions, WindowFrame, WindowFrameBound, WindowSpec, WindowType, With,
    WithFill,
};

use crate::SqlIdent;

/// Trait for comparing AST nodes for equality that takes into account SQL identifier case comparison rules and
/// superfluous nesting of expressions in parens, e.g. `(foo)` versus `foo`.
///
/// Otherwise it works exactly like [`Eq`].
///
/// # Why is this required?
///
/// 1. To robustly compare expressions in projections and `GROUP BY` clauses to accurately deternine which projection
/// columns should be aggregated.
///
/// 2. To encapsulate all of the `SqlIdent` logic (future work).
///
/// # TODO: generate this code
pub(crate) trait SyntacticEq {
    fn syntactic_eq(&self, other: &Self) -> bool;
}

impl SyntacticEq for Ident {
    fn syntactic_eq(&self, other: &Self) -> bool {
        SqlIdent(self) == SqlIdent(other)
    }
}

impl SyntacticEq for ObjectName {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self(idents_lhs) = self;
        let Self(idents_rhs) = other;

        idents_lhs.syntactic_eq(idents_rhs)
    }
}

impl<T: SyntacticEq> SyntacticEq for Vec<T> {
    fn syntactic_eq(&self, other: &Vec<T>) -> bool {
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

impl<T: SyntacticEq> SyntacticEq for Box<T> {
    fn syntactic_eq(&self, other: &Box<T>) -> bool {
        self.as_ref().syntactic_eq(other)
    }
}

impl SyntacticEq for MapAccessKey {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            key: key_lhs,
            syntax: syntax_lhs,
        } = self;
        let Self {
            key: key_rhs,
            syntax: syntax_rhs,
        } = other;

        key_lhs.syntactic_eq(key_rhs) && syntax_lhs == syntax_rhs
    }
}

impl SyntacticEq for Function {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            name: name_lhs,
            parameters: parameters_lhs,
            args: args_lhs,
            filter: filter_lhs,
            null_treatment: null_treatment_lhs,
            over: over_lhs,
            within_group: within_group_lhs,
        } = self;
        let Self {
            name: name_rhs,
            parameters: parameters_rhs,
            args: args_rhs,
            filter: filter_rhs,
            null_treatment: null_treatment_rhs,
            over: over_rhs,
            within_group: within_group_rhs,
        } = other;

        name_lhs.syntactic_eq(name_rhs)
            && parameters_lhs.syntactic_eq(parameters_rhs)
            && args_lhs.syntactic_eq(args_rhs)
            && filter_lhs.syntactic_eq(filter_rhs)
            && null_treatment_lhs == null_treatment_rhs
            && over_lhs.syntactic_eq(over_rhs)
            && within_group_lhs.syntactic_eq(within_group_rhs)
    }
}

impl SyntacticEq for WindowType {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (WindowType::WindowSpec(window_spec_lhs), WindowType::WindowSpec(window_spec_rhs)) => {
                window_spec_lhs.syntactic_eq(window_spec_rhs)
            }
            (WindowType::NamedWindow(ident_lhs), WindowType::NamedWindow(ident_rhs)) => {
                ident_lhs.syntactic_eq(ident_rhs)
            }
            _ => false,
        }
    }
}

impl SyntacticEq for FunctionArguments {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (FunctionArguments::None, FunctionArguments::None) => true,
            (FunctionArguments::Subquery(query_lhs), FunctionArguments::Subquery(query_rhs)) => {
                query_lhs.syntactic_eq(query_rhs)
            }
            (
                FunctionArguments::List(function_argument_list_lhs),
                FunctionArguments::List(function_argument_list_rhs),
            ) => function_argument_list_lhs.syntactic_eq(function_argument_list_rhs),
            _ => false,
        }
    }
}

impl SyntacticEq for FunctionArgumentList {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            duplicate_treatment: duplicate_treatment_lhs,
            args: args_lhs,
            clauses: clauses_lhs,
        } = self;
        let Self {
            duplicate_treatment: duplicate_treatment_rhs,
            args: args_rhs,
            clauses: clauses_rhs,
        } = other;

        duplicate_treatment_lhs == duplicate_treatment_rhs
            && args_lhs.syntactic_eq(args_rhs)
            && clauses_lhs.syntactic_eq(clauses_rhs)
    }
}

impl SyntacticEq for FunctionArgumentClause {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                FunctionArgumentClause::IgnoreOrRespectNulls(null_treatment_lhs),
                FunctionArgumentClause::IgnoreOrRespectNulls(null_treatment_rhs),
            ) => null_treatment_lhs == null_treatment_rhs,
            (
                FunctionArgumentClause::OrderBy(order_by_exprs_lhs),
                FunctionArgumentClause::OrderBy(order_by_exprs_rhs),
            ) => order_by_exprs_lhs.syntactic_eq(order_by_exprs_rhs),
            (FunctionArgumentClause::Limit(expr_lhs), FunctionArgumentClause::Limit(expr_rhs)) => {
                expr_lhs.syntactic_eq(expr_rhs)
            }
            (
                FunctionArgumentClause::OnOverflow(list_agg_on_overflow_lhs),
                FunctionArgumentClause::OnOverflow(list_agg_on_overflow_rhs),
            ) => list_agg_on_overflow_lhs.syntactic_eq(list_agg_on_overflow_rhs),
            (
                FunctionArgumentClause::Having(having_bound_lhs),
                FunctionArgumentClause::Having(having_bound_rhs),
            ) => having_bound_lhs.syntactic_eq(having_bound_rhs),
            (
                FunctionArgumentClause::Separator(value_lhs),
                FunctionArgumentClause::Separator(value_rhs),
            ) => value_lhs == value_rhs,
            _ => false,
        }
    }
}

impl SyntacticEq for HavingBound {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self(kind_lhs, expr_lhs) = self;
        let Self(kind_rhs, expr_rhs) = other;

        kind_lhs == kind_rhs && expr_lhs.syntactic_eq(expr_rhs)
    }
}

impl SyntacticEq for ListAggOnOverflow {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (ListAggOnOverflow::Error, ListAggOnOverflow::Error) => todo!(),
            (
                ListAggOnOverflow::Truncate {
                    filler: filler_lhs,
                    with_count: with_count_lhs,
                },
                ListAggOnOverflow::Truncate {
                    filler: filler_rhs,
                    with_count: with_count_rhs,
                },
            ) => filler_lhs.syntactic_eq(filler_rhs) && with_count_lhs == with_count_rhs,
            _ => false,
        }
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
            body: body_lhs,
            order_by: order_by_lhs,
            limit: limit_lhs,
            offset: offset_lhs,
            fetch: fetch_lhs,
            with: with_lhs,
            limit_by: limit_by_lhs,
            locks: locks_lhs,
            for_clause: for_clause_lhs,
            settings: settings_lhs,
            format_clause: format_clause_lhs,
        } = self;

        let Query {
            body: body_rhs,
            order_by: order_by_rhs,
            limit: limit_rhs,
            offset: offset_rhs,
            fetch: fetch_rhs,
            with: with_rhs,
            limit_by: limit_by_rhs,
            locks: locks_rhs,
            for_clause: for_clause_rhs,
            settings: settings_rhs,
            format_clause: format_clause_rhs,
        } = other;

        body_lhs.syntactic_eq(body_rhs)
            && order_by_lhs.syntactic_eq(order_by_rhs)
            && limit_lhs.syntactic_eq(limit_rhs)
            && offset_lhs.syntactic_eq(offset_rhs)
            && fetch_lhs.syntactic_eq(fetch_rhs)
            && with_lhs.syntactic_eq(with_rhs)
            && limit_by_lhs.syntactic_eq(limit_by_rhs)
            && locks_lhs.syntactic_eq(locks_rhs)
            && for_clause_lhs == for_clause_rhs
            && settings_lhs.syntactic_eq(settings_rhs)
            && format_clause_lhs.syntactic_eq(format_clause_rhs)
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
            (SetExpr::Select(select_lhs), SetExpr::Select(select_rhs)) => {
                select_lhs.syntactic_eq(select_rhs)
            }
            (SetExpr::Query(query_lhs), SetExpr::Query(query_rhs)) => {
                query_lhs.syntactic_eq(query_rhs)
            }
            (
                SetExpr::SetOperation {
                    op: op_lhs,
                    left: left_lhs,
                    right: right_lhs,
                    set_quantifier: set_quantifier_lhs,
                },
                SetExpr::SetOperation {
                    op: op_rhs,
                    left: left_rhs,
                    right: right_rhs,
                    set_quantifier: set_quantifier_rhs,
                },
            ) => {
                op_lhs == op_rhs
                    && op_lhs == op_rhs
                    && left_lhs.syntactic_eq(left_rhs)
                    && right_lhs.syntactic_eq(right_rhs)
                    && set_quantifier_lhs == set_quantifier_rhs
            }
            (SetExpr::Values(values_lhs), SetExpr::Values(values_rhs)) => {
                values_lhs.syntactic_eq(values_rhs)
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
            distinct: distinct_lhs,
            top: top_lhs,
            projection: projection_lhs,
            from: from_lhs,
            selection: selection_lhs,
            group_by: group_by_lhs,
            having: having_lhs,
            lateral_views: lateral_views_lhs,
            top_before_distinct: top_before_distinct_lhs,
            into: into_lhs,
            prewhere: prewhere_lhs,
            cluster_by: cluster_by_lhs,
            distribute_by: distribute_by_lhs,
            sort_by: sort_by_lhs,
            named_window: named_window_lhs,
            qualify: qualify_lhs,
            window_before_qualify: window_before_qualify_lhs,
            value_table_mode: value_table_mode_lhs,
            connect_by: connect_by_lhs,
        } = self;

        let Select {
            distinct: distinct_rhs,
            top: top_rhs,
            projection: projection_rhs,
            from: from_rhs,
            selection: selection_rhs,
            group_by: group_by_rhs,
            having: having_rhs,
            lateral_views: lateral_views_rhs,
            top_before_distinct: top_before_distinct_rhs,
            into: into_rhs,
            prewhere: prewhere_rhs,
            cluster_by: cluster_by_rhs,
            distribute_by: distribute_by_rhs,
            sort_by: sort_by_rhs,
            named_window: named_window_rhs,
            qualify: qualify_rhs,
            window_before_qualify: window_before_qualify_rhs,
            value_table_mode: value_table_mode_rhs,
            connect_by: connect_by_rhs,
        } = other;

        distinct_lhs.syntactic_eq(distinct_rhs)
            && top_lhs.syntactic_eq(top_rhs)
            && projection_lhs.syntactic_eq(projection_rhs)
            && from_lhs.syntactic_eq(from_rhs)
            && selection_lhs.syntactic_eq(selection_rhs)
            && group_by_lhs.syntactic_eq(group_by_rhs)
            && having_lhs.syntactic_eq(having_rhs)
            && lateral_views_lhs.syntactic_eq(lateral_views_rhs)
            && top_before_distinct_lhs == top_before_distinct_rhs
            && into_lhs.syntactic_eq(into_rhs)
            && prewhere_lhs.syntactic_eq(prewhere_rhs)
            && cluster_by_lhs.syntactic_eq(cluster_by_rhs)
            && distribute_by_lhs.syntactic_eq(distribute_by_rhs)
            && sort_by_lhs.syntactic_eq(sort_by_rhs)
            && named_window_lhs.syntactic_eq(named_window_rhs)
            && qualify_lhs.syntactic_eq(qualify_rhs)
            && window_before_qualify_lhs == window_before_qualify_rhs
            && value_table_mode_lhs == value_table_mode_rhs
            && connect_by_lhs.syntactic_eq(connect_by_rhs)
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
                    && alias_lhs.syntactic_eq(alias_rhs)
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
            ) => match_recognize_patterns_lhs.syntactic_eq(match_recognize_patterns_rhs),
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
            (Distinct::On(exprs_lhs), Distinct::On(exprs_rhs)) => exprs_lhs.syntactic_eq(exprs_rhs),
            _ => false,
        }
    }
}

impl SyntacticEq for Top {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            percent: percent_lhs,
            with_ties: with_ties_lhs,
            quantity: quantity_lhs,
        } = self;

        let Self {
            percent: percent_rhs,
            with_ties: with_ties_rhs,
            quantity: quantity_rhs,
        } = other;

        percent_lhs == percent_rhs
            && with_ties_lhs == with_ties_rhs
            && quantity_lhs.syntactic_eq(quantity_rhs)
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
        let Self {
            ident: ident_lhs,
            alias: alias_lhs,
        } = self;
        let Self {
            ident: ident_rhs,
            alias: alias_rhs,
        } = other;

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
            (TopQuantity::Expr(expr_lhs), TopQuantity::Expr(expr_rhs)) => {
                expr_lhs.syntactic_eq(expr_rhs)
            }
            (TopQuantity::Constant(c_lhs), TopQuantity::Constant(c_rhs)) => c_lhs == c_rhs,
            _ => false,
        }
    }
}

impl SyntacticEq for ReplaceSelectItem {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self { items: items_lhs } = self;
        let Self { items: items_rhs } = other;

        items_lhs.syntactic_eq(items_rhs)
    }
}

impl SyntacticEq for ReplaceSelectElement {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            expr: expr_lhs,
            column_name: column_name_lhs,
            as_keyword: as_keyword_lhs,
        } = self;
        let Self {
            expr: expr_rhs,
            column_name: column_name_rhs,
            as_keyword: as_keyword_rhs,
        } = other;

        expr_lhs.syntactic_eq(expr_rhs)
            && column_name_lhs.syntactic_eq(column_name_rhs)
            && as_keyword_lhs == as_keyword_rhs
    }
}

impl SyntacticEq for DateTimeField {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Week(ident_lhs), Self::Week(ident_rhs)) => ident_lhs.syntactic_eq(ident_rhs),
            (Self::Custom(ident_lhs), Self::Custom(ident_rhs)) => ident_lhs.syntactic_eq(ident_rhs),
            _ => mem::discriminant(self) == mem::discriminant(other),
        }
    }
}

impl SyntacticEq for CeilFloorKind {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (
                CeilFloorKind::DateTimeField(date_time_field_lhs),
                CeilFloorKind::DateTimeField(date_time_field_rhs),
            ) => date_time_field_lhs.syntactic_eq(date_time_field_rhs),
            (CeilFloorKind::Scale(value_lhs), CeilFloorKind::Scale(value_rhs)) => {
                value_lhs == value_rhs
            }
            _ => false,
        }
    }
}

impl SyntacticEq for StructField {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            field_name: field_name_lhs,
            field_type: field_type_lhs,
        } = self;
        let Self {
            field_name: field_name_rhs,
            field_type: field_type_rhs,
        } = other;

        field_name_lhs.syntactic_eq(field_name_rhs) && field_type_lhs == field_type_rhs
    }
}

impl SyntacticEq for DictionaryField {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            key: key_lhs,
            value: value_lhs,
        } = self;
        let Self {
            key: key_rhs,
            value: value_rhs,
        } = other;

        key_lhs.syntactic_eq(key_rhs) && value_lhs.syntactic_eq(value_rhs)
    }
}

impl SyntacticEq for Map {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            entries: entries_lhs,
        } = self;
        let Self {
            entries: entries_rhs,
        } = other;

        entries_lhs.syntactic_eq(entries_rhs)
    }
}

impl SyntacticEq for MapEntry {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            key: key_lhs,
            value: value_lhs,
        } = self;
        let Self {
            key: key_rhs,
            value: value_rhs,
        } = other;

        key_lhs.syntactic_eq(key_rhs) && value_lhs.syntactic_eq(value_rhs)
    }
}

impl SyntacticEq for Subscript {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (Subscript::Index { index: index_lhs }, Subscript::Index { index: index_rhs }) => {
                index_lhs.syntactic_eq(index_rhs)
            }
            (
                Subscript::Slice {
                    lower_bound: lower_bound_lhs,
                    upper_bound: upper_bound_lhs,
                    stride: stride_lhs,
                },
                Subscript::Slice {
                    lower_bound: lower_bound_rhs,
                    upper_bound: upper_bound_rhs,
                    stride: stride_rhs,
                },
            ) => {
                lower_bound_lhs.syntactic_eq(lower_bound_rhs)
                    && upper_bound_lhs.syntactic_eq(upper_bound_rhs)
                    && stride_lhs.syntactic_eq(stride_rhs)
            }
            _ => false,
        }
    }
}

impl SyntacticEq for Array {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            elem: elem_lhs,
            named: named_lhs,
        } = self;
        let Self {
            elem: elem_rhs,
            named: named_rhs,
        } = other;

        elem_lhs.syntactic_eq(elem_rhs) && *named_lhs == *named_rhs
    }
}

impl SyntacticEq for Interval {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            value: value_lhs,
            leading_field: leading_field_lhs,
            leading_precision: leading_precision_lhs,
            last_field: last_field_lhs,
            fractional_seconds_precision: fractional_seconds_precision_lhs,
        } = self;
        let Self {
            value: value_rhs,
            leading_field: leading_field_rhs,
            leading_precision: leading_precision_rhs,
            last_field: last_field_rhs,
            fractional_seconds_precision: fractional_seconds_precision_rhs,
        } = other;

        value_lhs.syntactic_eq(value_rhs)
            && leading_field_lhs.syntactic_eq(leading_field_rhs)
            && leading_precision_lhs == leading_precision_rhs
            && last_field_lhs.syntactic_eq(last_field_rhs)
            && fractional_seconds_precision_lhs == fractional_seconds_precision_rhs
    }
}

impl SyntacticEq for LambdaFunction {
    fn syntactic_eq(&self, other: &Self) -> bool {
        let Self {
            params: params_lhs,
            body: body_lhs,
        } = self;
        let Self {
            params: params_rhs,
            body: body_rhs,
        } = other;

        params_lhs.syntactic_eq(params_rhs) && body_lhs.syntactic_eq(body_rhs)
    }
}

impl<T: SyntacticEq> SyntacticEq for OneOrManyWithParens<T> {
    fn syntactic_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (OneOrManyWithParens::One(item_lhs), OneOrManyWithParens::One(item_rhs)) => {
                item_lhs.syntactic_eq(item_rhs)
            }
            (OneOrManyWithParens::Many(items_lhs), OneOrManyWithParens::Many(items_rhs)) => {
                items_lhs.syntactic_eq(items_rhs)
            }
            _ => false,
        }
    }
}

impl SyntacticEq for Expr {
    fn syntactic_eq(&self, other: &Self) -> bool {
        // Expr::Nested(_) requires special handling because the parens are superfluous when it comes to equality.
        match (self, other) {
            (Expr::Nested(expr_lhs), expr_rhs) => return (&**expr_lhs).syntactic_eq(expr_rhs),
            (expr_lhs, Expr::Nested(expr_rhs)) => return expr_lhs.syntactic_eq(expr_rhs),
            _ => {}
        }

        // If the discriminants are different then the nodes are different, so we can bail out.
        if mem::discriminant(self) != mem::discriminant(other) {
            return false;
        }

        match (self, other) {
            (Expr::Identifier(ident_lhs), Expr::Identifier(ident_rhs)) => {
                ident_lhs.syntactic_eq(ident_rhs)
            }
            (Expr::CompoundIdentifier(idents_lhs), Expr::CompoundIdentifier(idents_rhs)) => {
                idents_lhs.syntactic_eq(idents_rhs)
            }
            (
                Expr::JsonAccess {
                    value: value_lhs,
                    path: path_lhs,
                },
                Expr::JsonAccess {
                    value: value_rhs,
                    path: path_rhs,
                },
            ) => value_lhs.syntactic_eq(value_rhs) && path_lhs.syntactic_eq(path_rhs),
            (
                Expr::CompositeAccess {
                    expr: expr_lhs,
                    key: key_lhs,
                },
                Expr::CompositeAccess {
                    expr: expr_rhs,
                    key: key_rhs,
                },
            ) => expr_lhs.syntactic_eq(expr_rhs) && key_lhs.syntactic_eq(key_rhs),
            (Expr::IsFalse(expr_lhs), Expr::IsFalse(expr_rhs)) => expr_lhs.syntactic_eq(expr_rhs),
            (Expr::IsNotFalse(expr_lhs), Expr::IsNotFalse(expr_rhs)) => {
                expr_lhs.syntactic_eq(expr_rhs)
            }
            (Expr::IsTrue(expr_lhs), Expr::IsTrue(expr_rhs)) => expr_lhs.syntactic_eq(expr_rhs),
            (Expr::IsNotTrue(expr_lhs), Expr::IsNotTrue(expr_rhs)) => {
                expr_lhs.syntactic_eq(expr_rhs)
            }
            (Expr::IsNull(expr_lhs), Expr::IsNull(expr_rhs)) => expr_lhs.syntactic_eq(expr_rhs),
            (Expr::IsNotNull(expr_lhs), Expr::IsNotNull(expr_rhs)) => {
                expr_lhs.syntactic_eq(expr_rhs)
            }
            (Expr::IsUnknown(expr_lhs), Expr::IsUnknown(expr_rhs)) => {
                expr_lhs.syntactic_eq(expr_rhs)
            }
            (Expr::IsNotUnknown(expr_lhs), Expr::IsNotUnknown(expr_rhs)) => {
                expr_lhs.syntactic_eq(expr_rhs)
            }
            (
                Expr::IsDistinctFrom(expr_lhs, expr1_lhs),
                Expr::IsDistinctFrom(expr_rhs, expr1_rhs),
            ) => expr_lhs.syntactic_eq(expr_rhs) && expr1_lhs.syntactic_eq(expr1_rhs),
            (
                Expr::IsNotDistinctFrom(expr_lhs, expr1_lhs),
                Expr::IsNotDistinctFrom(expr_rhs, expr1_rhs),
            ) => expr_lhs.syntactic_eq(expr_rhs) && expr1_lhs.syntactic_eq(expr1_rhs),
            (
                Expr::InList {
                    expr: expr_lhs,
                    list: list_lhs,
                    negated: negated_lhs,
                },
                Expr::InList {
                    expr: expr_rhs,
                    list: list_rhs,
                    negated: negated_rhs,
                },
            ) => {
                expr_lhs.syntactic_eq(expr_rhs)
                    && list_lhs
                        .iter()
                        .zip(list_rhs.iter())
                        .all(|(l, r)| l.syntactic_eq(r))
                    && negated_lhs == negated_rhs
            }
            (
                Expr::InSubquery {
                    expr: expr_lhs,
                    subquery: subquery_lhs,
                    negated: negated_lhs,
                },
                Expr::InSubquery {
                    expr: expr_rhs,
                    subquery: subquery_rhs,
                    negated: negated_rhs,
                },
            ) => {
                expr_lhs.syntactic_eq(expr_rhs)
                    && subquery_lhs.syntactic_eq(subquery_rhs)
                    && negated_lhs == negated_rhs
            }
            (
                Expr::InUnnest {
                    expr: expr_lhs,
                    array_expr: array_expr_lhs,
                    negated: negated_lhs,
                },
                Expr::InUnnest {
                    expr: expr_rhs,
                    array_expr: array_expr_rhs,
                    negated: negated_rhs,
                },
            ) => {
                expr_lhs.syntactic_eq(expr_rhs)
                    && array_expr_lhs.syntactic_eq(array_expr_rhs)
                    && negated_lhs == negated_rhs
            }
            (
                Expr::Between {
                    expr: expr_lhs,
                    negated: negated_lhs,
                    low: low_lhs,
                    high: high_lhs,
                },
                Expr::Between {
                    expr: expr_rhs,
                    negated: negated_rhs,
                    low: low_rhs,
                    high: high_rhs,
                },
            ) => {
                expr_lhs.syntactic_eq(expr_rhs)
                    && negated_lhs == negated_rhs
                    && low_lhs.syntactic_eq(low_rhs)
                    && high_lhs.syntactic_eq(high_rhs)
            }
            (
                Expr::BinaryOp {
                    left: left_lhs,
                    op: op_lhs,
                    right: right_lhs,
                },
                Expr::BinaryOp {
                    left: left_rhs,
                    op: op_rhs,
                    right: right_rhs,
                },
            ) => {
                left_lhs.syntactic_eq(left_rhs)
                    && right_lhs.syntactic_eq(right_rhs)
                    && op_lhs == op_rhs
            }
            (
                Expr::Like {
                    negated: negated_lhs,
                    any: any_lhs,
                    expr: expr_lhs,
                    pattern: pattern_lhs,
                    escape_char: escape_char_lhs,
                },
                Expr::Like {
                    negated: negated_rhs,
                    any: any_rhs,
                    expr: expr_rhs,
                    pattern: pattern_rhs,
                    escape_char: escape_char_rhs,
                },
            ) => {
                negated_lhs == negated_rhs
                    && any_lhs == any_rhs
                    && expr_lhs.syntactic_eq(expr_rhs)
                    && pattern_lhs.syntactic_eq(pattern_rhs)
                    && escape_char_lhs == escape_char_rhs
            }
            (
                Expr::ILike {
                    negated: negated_lhs,
                    any: any_lhs,
                    expr: expr_lhs,
                    pattern: pattern_lhs,
                    escape_char: escape_char_lhs,
                },
                Expr::ILike {
                    negated: negated_rhs,
                    any: any_rhs,
                    expr: expr_rhs,
                    pattern: pattern_rhs,
                    escape_char: escape_char_rhs,
                },
            ) => {
                negated_lhs == negated_rhs
                    && any_lhs == any_rhs
                    && expr_lhs.syntactic_eq(expr_rhs)
                    && pattern_lhs.syntactic_eq(pattern_rhs)
                    && escape_char_lhs == escape_char_rhs
            }
            (
                Expr::SimilarTo {
                    negated: negated_lhs,
                    expr: expr_lhs,
                    pattern: pattern_lhs,
                    escape_char: escape_char_lhs,
                },
                Expr::SimilarTo {
                    negated: negated_rhs,
                    expr: expr_rhs,
                    pattern: pattern_rhs,
                    escape_char: escape_char_rhs,
                },
            ) => {
                negated_lhs == negated_rhs
                    && expr_lhs.syntactic_eq(expr_rhs)
                    && pattern_lhs.syntactic_eq(pattern_rhs)
                    && escape_char_lhs == escape_char_rhs
            }
            (
                Expr::RLike {
                    negated: negated_lhs,
                    expr: expr_lhs,
                    pattern: pattern_lhs,
                    regexp: regexp_lhs,
                },
                Expr::RLike {
                    negated: negated_rhs,
                    expr: expr_rhs,
                    pattern: pattern_rhs,
                    regexp: regexp_rhs,
                },
            ) => {
                negated_lhs == negated_rhs
                    && expr_lhs.syntactic_eq(expr_rhs)
                    && pattern_lhs.syntactic_eq(pattern_rhs)
                    && regexp_lhs == regexp_rhs
            }
            (
                Expr::AnyOp {
                    left: left_lhs,
                    compare_op: compare_op_lhs,
                    right: right_lhs,
                    is_some: is_some_lhs,
                },
                Expr::AnyOp {
                    left: left_rhs,
                    compare_op: compare_op_rhs,
                    right: right_rhs,
                    is_some: is_some_rhs,
                },
            ) => {
                left_lhs.syntactic_eq(left_rhs)
                    && compare_op_lhs == compare_op_rhs
                    && right_lhs.syntactic_eq(right_rhs)
                    && is_some_lhs == is_some_rhs
            }
            (
                Expr::AllOp {
                    left: left_lhs,
                    compare_op: compare_op_lhs,
                    right: right_lhs,
                },
                Expr::AllOp {
                    left: left_rhs,
                    compare_op: compare_op_rhs,
                    right: right_rhs,
                },
            ) => {
                left_lhs.syntactic_eq(left_rhs)
                    && compare_op_lhs == compare_op_rhs
                    && right_lhs.syntactic_eq(right_rhs)
            }
            (
                Expr::UnaryOp {
                    op: op_lhs,
                    expr: expr_lhs,
                },
                Expr::UnaryOp {
                    op: op_rhs,
                    expr: expr_rhs,
                },
            ) => op_lhs == op_rhs && expr_lhs.syntactic_eq(expr_rhs),
            (
                Expr::Convert {
                    is_try: is_try_lhs,
                    expr: expr_lhs,
                    data_type: data_type_lhs,
                    charset: charset_lhs,
                    target_before_value: target_before_value_lhs,
                    styles: styles_lhs,
                },
                Expr::Convert {
                    is_try: is_try_rhs,
                    expr: expr_rhs,
                    data_type: data_type_rhs,
                    charset: charset_rhs,
                    target_before_value: target_before_value_rhs,
                    styles: styles_rhs,
                },
            ) => {
                is_try_lhs == is_try_rhs
                    && expr_lhs.syntactic_eq(expr_rhs)
                    && data_type_lhs == data_type_rhs
                    && charset_lhs.syntactic_eq(charset_rhs)
                    && target_before_value_lhs == target_before_value_rhs
                    && styles_lhs.syntactic_eq(styles_rhs)
            }
            (
                Expr::Cast {
                    kind: kind_lhs,
                    expr: expr_lhs,
                    data_type: data_type_lhs,
                    format: format_lhs,
                },
                Expr::Cast {
                    kind: kind_rhs,
                    expr: expr_rhs,
                    data_type: data_type_rhs,
                    format: format_rhs,
                },
            ) => {
                kind_lhs == kind_rhs
                    && expr_lhs.syntactic_eq(expr_rhs)
                    && data_type_lhs == data_type_rhs
                    && format_lhs == format_rhs
            }
            (
                Expr::AtTimeZone {
                    timestamp: timestamp_lhs,
                    time_zone: time_zone_lhs,
                },
                Expr::AtTimeZone {
                    timestamp: timestamp_rhs,
                    time_zone: time_zone_rhs,
                },
            ) => {
                timestamp_lhs.syntactic_eq(timestamp_rhs)
                    && time_zone_lhs.syntactic_eq(time_zone_rhs)
            }
            (
                Expr::Extract {
                    field: field_lhs,
                    syntax: syntax_lhs,
                    expr: expr_lhs,
                },
                Expr::Extract {
                    field: field_rhs,
                    syntax: syntax_rhs,
                    expr: expr_rhs,
                },
            ) => {
                field_lhs.syntactic_eq(field_rhs)
                    && syntax_lhs == syntax_rhs
                    && expr_lhs.syntactic_eq(expr_rhs)
            }
            (
                Expr::Ceil {
                    expr: expr_lhs,
                    field: field_lhs,
                },
                Expr::Ceil {
                    expr: expr_rhs,
                    field: field_rhs,
                },
            ) => expr_lhs.syntactic_eq(expr_rhs) && field_lhs.syntactic_eq(field_rhs),
            (
                Expr::Floor {
                    expr: expr_lhs,
                    field: field_lhs,
                },
                Expr::Floor {
                    expr: expr_rhs,
                    field: field_rhs,
                },
            ) => expr_lhs.syntactic_eq(expr_rhs) && field_lhs.syntactic_eq(field_rhs),
            (
                Expr::Position {
                    expr: expr_lhs,
                    r#in: in_lhs,
                },
                Expr::Position {
                    expr: expr_rhs,
                    r#in: in_rhs,
                },
            ) => expr_lhs.syntactic_eq(expr_rhs) && in_lhs.syntactic_eq(in_rhs),
            (
                Expr::Substring {
                    expr: expr_lhs,
                    substring_from: substring_from_lhs,
                    substring_for: substring_for_lhs,
                    special: special_lhs,
                },
                Expr::Substring {
                    expr: expr_rhs,
                    substring_from: substring_from_rhs,
                    substring_for: substring_for_rhs,
                    special: special_rhs,
                },
            ) => {
                expr_lhs.syntactic_eq(expr_rhs)
                    && substring_from_lhs.syntactic_eq(substring_from_rhs)
                    && substring_for_lhs.syntactic_eq(substring_for_rhs)
                    && special_lhs == special_rhs
            }
            (
                Expr::Trim {
                    expr: expr_lhs,
                    trim_where: trim_where_lhs,
                    trim_what: trim_what_lhs,
                    trim_characters: trim_characters_lhs,
                },
                Expr::Trim {
                    expr: expr_rhs,
                    trim_where: trim_where_rhs,
                    trim_what: trim_what_rhs,
                    trim_characters: trim_characters_rhs,
                },
            ) => {
                expr_lhs.syntactic_eq(expr_rhs)
                    && trim_where_lhs == trim_where_rhs
                    && trim_what_lhs.syntactic_eq(trim_what_rhs)
                    && trim_characters_lhs.syntactic_eq(trim_characters_rhs)
            }
            (
                Expr::Overlay {
                    expr: expr_lhs,
                    overlay_what: overlay_what_lhs,
                    overlay_from: overlay_from_lhs,
                    overlay_for: overlay_for_lhs,
                },
                Expr::Overlay {
                    expr: expr_rhs,
                    overlay_what: overlay_what_rhs,
                    overlay_from: overlay_from_rhs,
                    overlay_for: overlay_for_rhs,
                },
            ) => {
                expr_lhs.syntactic_eq(expr_rhs)
                    && overlay_what_lhs.syntactic_eq(overlay_what_rhs)
                    && overlay_from_lhs.syntactic_eq(overlay_from_rhs)
                    && overlay_for_lhs.syntactic_eq(overlay_for_rhs)
            }
            (
                Expr::Collate {
                    expr: expr_lhs,
                    collation: collation_lhs,
                },
                Expr::Collate {
                    expr: expr_rhs,
                    collation: collation_rhs,
                },
            ) => expr_lhs.syntactic_eq(expr_rhs) && collation_lhs.syntactic_eq(collation_rhs),
            (Expr::Value(value_lhs), Expr::Value(value_rhs)) => value_lhs == value_rhs,
            (
                Expr::IntroducedString {
                    introducer: introducer_lhs,
                    value: value_lhs,
                },
                Expr::IntroducedString {
                    introducer: introducer_rhs,
                    value: value_rhs,
                },
            ) => introducer_lhs == introducer_rhs && value_lhs == value_rhs,
            (
                Expr::TypedString {
                    data_type: data_type_lhs,
                    value: value_lhs,
                },
                Expr::TypedString {
                    data_type: data_type_rhs,
                    value: value_rhs,
                },
            ) => data_type_lhs == data_type_rhs && value_lhs == value_rhs,
            (
                Expr::MapAccess {
                    column: column_lhs,
                    keys: keys_lhs,
                },
                Expr::MapAccess {
                    column: column_rhs,
                    keys: keys_rhs,
                },
            ) => column_lhs.syntactic_eq(column_rhs) && keys_lhs.syntactic_eq(keys_rhs),
            (Expr::Function(function_lhs), Expr::Function(function_rhs)) => {
                function_lhs.syntactic_eq(function_rhs)
            }
            (
                Expr::Case {
                    operand: operand_lhs,
                    conditions: conditions_lhs,
                    results: results_lhs,
                    else_result: else_result_lhs,
                },
                Expr::Case {
                    operand: operand_rhs,
                    conditions: conditions_rhs,
                    results: results_rhs,
                    else_result: else_result_rhs,
                },
            ) => {
                operand_lhs.syntactic_eq(operand_rhs)
                    && conditions_lhs.syntactic_eq(conditions_rhs)
                    && results_lhs.syntactic_eq(results_rhs)
                    && else_result_lhs.syntactic_eq(else_result_rhs)
            }
            (
                Expr::Exists {
                    subquery: subquery_lhs,
                    negated: negated_lhs,
                },
                Expr::Exists {
                    subquery: subquery_rhs,
                    negated: negated_rhs,
                },
            ) => subquery_lhs.syntactic_eq(subquery_rhs) && negated_lhs == negated_rhs,
            (Expr::Subquery(query_lhs), Expr::Subquery(query_rhs)) => {
                query_lhs.syntactic_eq(query_rhs)
            }
            (Expr::GroupingSets(items_lhs), Expr::GroupingSets(items_rhs)) => {
                items_lhs.syntactic_eq(items_rhs)
            }
            (Expr::Cube(items_lhs), Expr::Cube(items_rhs)) => items_lhs.syntactic_eq(items_rhs),
            (Expr::Rollup(items_lhs), Expr::Rollup(items_rhs)) => items_lhs.syntactic_eq(items_rhs),
            (Expr::Tuple(exprs_lhs), Expr::Tuple(exprs_rhs)) => exprs_lhs.syntactic_eq(exprs_rhs),
            (
                Expr::Struct {
                    values: values_lhs,
                    fields: fields_lhs,
                },
                Expr::Struct {
                    values: values_rhs,
                    fields: fields_rhs,
                },
            ) => values_lhs.syntactic_eq(values_rhs) && fields_lhs.syntactic_eq(fields_rhs),
            (
                Expr::Named {
                    expr: expr_lhs,
                    name: name_lhs,
                },
                Expr::Named {
                    expr: expr_rhs,
                    name: name_rhs,
                },
            ) => expr_lhs.syntactic_eq(expr_rhs) && name_lhs.syntactic_eq(name_rhs),
            (Expr::Dictionary(dictionary_fields_lhs), Expr::Dictionary(dictionary_fields_rhs)) => {
                dictionary_fields_lhs.syntactic_eq(dictionary_fields_rhs)
            }
            (Expr::Map(map_lhs), Expr::Map(map_rhs)) => map_lhs.syntactic_eq(map_rhs),
            (
                Expr::Subscript {
                    expr: expr_lhs,
                    subscript: subscript_lhs,
                },
                Expr::Subscript {
                    expr: expr_rhs,
                    subscript: subscript_rhs,
                },
            ) => expr_lhs.syntactic_eq(expr_rhs) && subscript_lhs.syntactic_eq(subscript_rhs),
            (Expr::Array(array_lhs), Expr::Array(array_rhs)) => array_lhs.syntactic_eq(array_rhs),
            (Expr::Interval(interval_lhs), Expr::Interval(interval_rhs)) => {
                interval_lhs.syntactic_eq(interval_rhs)
            }
            (
                Expr::MatchAgainst {
                    columns: columns_lhs,
                    match_value: match_value_lhs,
                    opt_search_modifier: opt_search_modifier_lhs,
                },
                Expr::MatchAgainst {
                    columns: columns_rhs,
                    match_value: match_value_rhs,
                    opt_search_modifier: opt_search_modifier_rhs,
                },
            ) => {
                columns_lhs.syntactic_eq(columns_rhs)
                    && match_value_lhs == match_value_rhs
                    && opt_search_modifier_lhs == opt_search_modifier_rhs
            }
            (Expr::Wildcard, Expr::Wildcard) => true,
            (
                Expr::QualifiedWildcard(object_name_lhs),
                Expr::QualifiedWildcard(object_name_rhs),
            ) => object_name_lhs.syntactic_eq(object_name_rhs),
            (Expr::OuterJoin(expr_lhs), Expr::OuterJoin(expr_rhs)) => {
                expr_lhs.syntactic_eq(expr_rhs)
            }
            (Expr::Prior(expr_lhs), Expr::Prior(expr_rhs)) => expr_lhs.syntactic_eq(expr_rhs),
            (Expr::Lambda(lambda_function_lhs), Expr::Lambda(lambda_function_rhs)) => {
                lambda_function_lhs.syntactic_eq(lambda_function_rhs)
            }
            _ => false
        }
    }
}
