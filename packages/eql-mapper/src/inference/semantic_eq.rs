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

use crate::{ScopeTracker, SqlIdent};

/// Trait for comparing [`Expr`] (and types that are transitively reachable via a struct field or enum variant of
/// `Expr`) nodes for semantic equivalence.
///
/// This trait is required in order to accurately determine which projection columns require aggregation when there is a
/// `GROUP BY` clause present.
///
/// For example:
///
/// ```sql
/// -- the same identifier 'email' is used in both the SELECT projection and the GROUP BY clause
/// -- therefore 'name' must be aggregated
/// SELECT email, MIN(name) FROM users GROUP BY email;
///
/// -- the identifier 'eMaIl' us used in the SELECT projection and the compound identifier 'u.email' in used in the GROUP BY
/// -- therefore 'name' must be aggregated because due to SQL identifier comparison rules, unquoted identifiers are compared ignoring case.
/// SELECT eMaIl, MIN(name) FROM users GROUP BY email;
///
/// -- the identifier 'email' us used in the SELECT projection and the compound identifier 'u.email' in used in the GROUP BY
/// -- therefore 'name' must be aggregated
/// SELECT email, MIN(name) FROM users as u GROUP BY u.email;
/// ```
///
/// If we only support syntactic equality with [`std::cmp::Eq`] then we could only support the first case.
///
/// Two [`Expr`] nodes `LHS` and `RHS` (left hand side and right hand side, respectively) will be considered
/// semantically equal if (and only if):
///
/// 1. Both `LHS` & `RHS` are **identifiers** ([`Expr::Identifier`] or [`Expr::CompoundIdentifier`]) and they resolve to
///    the same [`crate::inference::unifier::Value`] in the same [`scope`](`ScopeTracker`).
///
/// 2. Both `LHS` & `RHS` are [`Expr::Wildcard`]
///
/// 3. Both `LHS` & `RHS` are [`Expr::QualifiedWildcard`]
///
/// 4. `LHS` is `Expr::Nested(lhs_inner)` and `lhs_inner` is semantically equal to `RHS`, or
///
/// 5. `RHS` is `Expr::Nested(rhs_inner)` and `rhs_inner` is semantically equal to `LHS`, or
///
/// 6. Both expressions are the same variant and are recursively semantically equal.
///
/// Every AST node type that is reachable via an [`Expr`] variant implements `SemanticEq` except where that type does
/// not own (transitively) an `Expr` and those fallback to [`std::cmp::Eq`].
pub(crate) trait SemanticEq<'ast> {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool;
}

/// The implementation for [`Ident`] is purely syntactic. Identifier resolution is handled in the implementation for
/// [`Expr`].
impl<'ast> SemanticEq<'ast> for Ident {
    fn semantic_eq(&self, other: &Self, _: &ScopeTracker<'ast>) -> bool {
        SqlIdent(self) == SqlIdent(other)
    }
}

/// The implementation for [`ObjectName`] is purely syntactic. Compound identifier resolution is handled in the
/// implementation for [`Expr`].
impl<'ast> SemanticEq<'ast> for ObjectName {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        self.0.len() == other.0.len()
            && self
                .0
                .iter()
                .zip(other.0.iter())
                .fold(true, |acc, (l, r)| acc && l.semantic_eq(r, scope))
    }
}

impl<'ast, T: SemanticEq<'ast>> SemanticEq<'ast> for Vec<T> {
    fn semantic_eq(&self, other: &Vec<T>, scope: &ScopeTracker<'ast>) -> bool {
        self.len() == other.len()
            && self
                .iter()
                .zip(other.iter())
                .fold(true, |acc, (l, r)| acc && l.semantic_eq(r, scope))
    }
}

impl<'ast, T: SemanticEq<'ast>> SemanticEq<'ast> for Option<T> {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (Some(l), Some(r)) => l.semantic_eq(r, scope),
            (None, None) => true,
            _ => false,
        }
    }
}

impl<'ast, T: SemanticEq<'ast>> SemanticEq<'ast> for Box<T> {
    fn semantic_eq(&self, other: &Box<T>, scope: &ScopeTracker<'ast>) -> bool {
        (**self).semantic_eq(&**other, scope)
    }
}

impl<'ast> SemanticEq<'ast> for MapAccessKey {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self {
            key: key_lhs,
            syntax: syntax_lhs,
        } = self;
        let Self {
            key: key_rhs,
            syntax: syntax_rhs,
        } = other;

        key_lhs.semantic_eq(key_rhs, scope) && syntax_lhs == syntax_rhs
    }
}

impl<'ast> SemanticEq<'ast> for Function {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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

        name_lhs.semantic_eq(name_rhs, scope)
            && parameters_lhs.semantic_eq(parameters_rhs, scope)
            && args_lhs.semantic_eq(args_rhs, scope)
            && filter_lhs.semantic_eq(filter_rhs, scope)
            && null_treatment_lhs == null_treatment_rhs
            && over_lhs.semantic_eq(over_rhs, scope)
            && within_group_lhs.semantic_eq(within_group_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for WindowType {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (WindowType::WindowSpec(window_spec_lhs), WindowType::WindowSpec(window_spec_rhs)) => {
                window_spec_lhs.semantic_eq(window_spec_rhs, scope)
            }
            (WindowType::NamedWindow(ident_lhs), WindowType::NamedWindow(ident_rhs)) => {
                ident_lhs.semantic_eq(ident_rhs, scope)
            }
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for FunctionArguments {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (FunctionArguments::None, FunctionArguments::None) => true,
            (FunctionArguments::Subquery(query_lhs), FunctionArguments::Subquery(query_rhs)) => {
                query_lhs.semantic_eq(query_rhs, scope)
            }
            (
                FunctionArguments::List(function_argument_list_lhs),
                FunctionArguments::List(function_argument_list_rhs),
            ) => function_argument_list_lhs.semantic_eq(function_argument_list_rhs, scope),
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for FunctionArgumentList {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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
            && args_lhs.semantic_eq(args_rhs, scope)
            && clauses_lhs.semantic_eq(clauses_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for FunctionArgumentClause {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (
                FunctionArgumentClause::IgnoreOrRespectNulls(null_treatment_lhs),
                FunctionArgumentClause::IgnoreOrRespectNulls(null_treatment_rhs),
            ) => null_treatment_lhs == null_treatment_rhs,
            (
                FunctionArgumentClause::OrderBy(order_by_exprs_lhs),
                FunctionArgumentClause::OrderBy(order_by_exprs_rhs),
            ) => order_by_exprs_lhs.semantic_eq(order_by_exprs_rhs, scope),
            (FunctionArgumentClause::Limit(expr_lhs), FunctionArgumentClause::Limit(expr_rhs)) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
            }
            (
                FunctionArgumentClause::OnOverflow(list_agg_on_overflow_lhs),
                FunctionArgumentClause::OnOverflow(list_agg_on_overflow_rhs),
            ) => list_agg_on_overflow_lhs.semantic_eq(list_agg_on_overflow_rhs, scope),
            (
                FunctionArgumentClause::Having(having_bound_lhs),
                FunctionArgumentClause::Having(having_bound_rhs),
            ) => having_bound_lhs.semantic_eq(having_bound_rhs, scope),
            (
                FunctionArgumentClause::Separator(value_lhs),
                FunctionArgumentClause::Separator(value_rhs),
            ) => value_lhs == value_rhs,
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for HavingBound {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self(kind_lhs, expr_lhs) = self;
        let Self(kind_rhs, expr_rhs) = other;

        kind_lhs == kind_rhs && expr_lhs.semantic_eq(expr_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for ListAggOnOverflow {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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
            ) => filler_lhs.semantic_eq(filler_rhs, scope) && with_count_lhs == with_count_rhs,
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for JsonPath {
    fn semantic_eq(&self, other: &Self, _scope: &ScopeTracker<'ast>) -> bool {
        self.path == other.path
    }
}

impl<'ast> SemanticEq<'ast> for Query {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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

        body_lhs.semantic_eq(body_rhs, scope)
            && order_by_lhs.semantic_eq(order_by_rhs, scope)
            && limit_lhs.semantic_eq(limit_rhs, scope)
            && offset_lhs.semantic_eq(offset_rhs, scope)
            && fetch_lhs.semantic_eq(fetch_rhs, scope)
            && with_lhs.semantic_eq(with_rhs, scope)
            && limit_by_lhs.semantic_eq(limit_by_rhs, scope)
            && locks_lhs.semantic_eq(locks_rhs, scope)
            && for_clause_lhs == for_clause_rhs
            && settings_lhs.semantic_eq(settings_rhs, scope)
            && format_clause_lhs.semantic_eq(format_clause_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for FormatClause {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (FormatClause::Identifier(lhs), FormatClause::Identifier(rhs)) => {
                rhs.semantic_eq(lhs, scope)
            }
            (FormatClause::Null, FormatClause::Null) => true,
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for Setting {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        self.key.semantic_eq(&other.key, scope) && self.value == other.value
    }
}

impl<'ast> SemanticEq<'ast> for LockClause {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        self.lock_type == other.lock_type
            && self.of.semantic_eq(&other.of, scope)
            && self.nonblock == other.nonblock
    }
}

impl<'ast> SemanticEq<'ast> for With {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        self.recursive == other.recursive && self.cte_tables.semantic_eq(&other.cte_tables, scope)
    }
}

impl<'ast> SemanticEq<'ast> for Cte {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        self.alias.semantic_eq(&other.alias, scope)
            && self.query.semantic_eq(&other.query, scope)
            && self.from.semantic_eq(&other.from, scope)
            && self.materialized == other.materialized
    }
}

impl<'ast> SemanticEq<'ast> for TableAlias {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        self.name.semantic_eq(&other.name, scope) && self.columns.semantic_eq(&other.columns, scope)
    }
}

impl<'ast> SemanticEq<'ast> for Fetch {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        self.with_ties == other.with_ties
            && self.percent == other.percent
            && self.quantity.semantic_eq(&other.quantity, scope)
    }
}

impl<'ast> SemanticEq<'ast> for OrderBy {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        self.exprs.semantic_eq(&other.exprs, scope) && self.interpolate.semantic_eq(&other.interpolate, scope)
    }
}

impl<'ast> SemanticEq<'ast> for OrderByExpr {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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

        expr_lhs.semantic_eq(expr_rhs, scope)
            && nulls_first_lhs == nulls_first_rhs
            && with_fill_lhs.semantic_eq(with_fill_rhs, scope)
            && asc_lhs == asc_rhs
    }
}

impl<'ast> SemanticEq<'ast> for WithFill {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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

        from_lhs.semantic_eq(from_rhs, scope)
            && to_lhs.semantic_eq(to_rhs, scope)
            && step_lhs.semantic_eq(step_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for Interpolate {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        self.exprs.semantic_eq(&other.exprs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for InterpolateExpr {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        self.column.semantic_eq(&other.column, scope) && self.expr.semantic_eq(&other.expr, scope)
    }
}

impl<'ast> SemanticEq<'ast> for Offset {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        self.value.semantic_eq(&other.value, scope) && self.rows == other.rows
    }
}

impl<'ast> SemanticEq<'ast> for SetExpr {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (SetExpr::Select(select_lhs), SetExpr::Select(select_rhs)) => {
                select_lhs.semantic_eq(select_rhs, scope)
            }
            (SetExpr::Query(query_lhs), SetExpr::Query(query_rhs)) => {
                query_lhs.semantic_eq(query_rhs, scope)
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
                    && left_lhs.semantic_eq(left_rhs, scope)
                    && right_lhs.semantic_eq(right_rhs, scope)
                    && set_quantifier_lhs == set_quantifier_rhs
            }
            (SetExpr::Values(values_lhs), SetExpr::Values(values_rhs)) => {
                values_lhs.semantic_eq(values_rhs, scope)
            }
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for Values {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self {
            explicit_row: explicit_row_lhs,
            rows: rows_lhs,
        } = self;
        let Self {
            explicit_row: explicit_row_rhs,
            rows: rows_rhs,
        } = other;

        explicit_row_lhs == explicit_row_rhs && rows_lhs.semantic_eq(rows_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for Select {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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

        distinct_lhs.semantic_eq(distinct_rhs, scope)
            && top_lhs.semantic_eq(top_rhs, scope)
            && projection_lhs.semantic_eq(projection_rhs, scope)
            && from_lhs.semantic_eq(from_rhs, scope)
            && selection_lhs.semantic_eq(selection_rhs, scope)
            && group_by_lhs.semantic_eq(group_by_rhs, scope)
            && having_lhs.semantic_eq(having_rhs, scope)
            && lateral_views_lhs.semantic_eq(lateral_views_rhs, scope)
            && top_before_distinct_lhs == top_before_distinct_rhs
            && into_lhs.semantic_eq(into_rhs, scope)
            && prewhere_lhs.semantic_eq(prewhere_rhs, scope)
            && cluster_by_lhs.semantic_eq(cluster_by_rhs, scope)
            && distribute_by_lhs.semantic_eq(distribute_by_rhs, scope)
            && sort_by_lhs.semantic_eq(sort_by_rhs, scope)
            && named_window_lhs.semantic_eq(named_window_rhs, scope)
            && qualify_lhs.semantic_eq(qualify_rhs, scope)
            && window_before_qualify_lhs == window_before_qualify_rhs
            && value_table_mode_lhs == value_table_mode_rhs
            && connect_by_lhs.semantic_eq(connect_by_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for ConnectBy {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self {
            condition: condition_lhs,
            relationships: relationships_lhs,
        } = self;
        let Self {
            condition: condition_rhs,
            relationships: relationships_rhs,
        } = other;

        condition_lhs.semantic_eq(condition_rhs, scope)
            && relationships_lhs.semantic_eq(relationships_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for NamedWindowDefinition {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self(ident_lhs, named_window_expr_lhs) = self;
        let Self(ident_rhs, named_window_expr_rhs) = other;

        ident_lhs.semantic_eq(ident_rhs, scope)
            && named_window_expr_lhs.semantic_eq(named_window_expr_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for NamedWindowExpr {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (NamedWindowExpr::NamedWindow(ident_lhs), NamedWindowExpr::NamedWindow(ident_rhs)) => {
                ident_lhs.semantic_eq(ident_rhs, scope)
            }
            (
                NamedWindowExpr::WindowSpec(window_spec_lhs),
                NamedWindowExpr::WindowSpec(window_spec_rhs),
            ) => window_spec_lhs.semantic_eq(window_spec_rhs, scope),
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for WindowSpec {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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

        window_name_lhs.semantic_eq(window_name_rhs, scope)
            && partition_by_lhs.semantic_eq(partition_by_rhs, scope)
            && order_by_lhs.semantic_eq(order_by_rhs, scope)
            && window_frame_lhs.semantic_eq(window_frame_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for WindowFrame {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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
            && start_bound_lhs.semantic_eq(start_bound_rhs, scope)
            && end_bound_lhs.semantic_eq(end_bound_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for WindowFrameBound {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (WindowFrameBound::CurrentRow, WindowFrameBound::CurrentRow) => true,
            (WindowFrameBound::Preceding(expr_lhs), WindowFrameBound::Preceding(expr_rhs)) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
            }
            (WindowFrameBound::Following(expr_lhs), WindowFrameBound::Following(expr_rhs)) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
            }
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for SelectInto {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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

        name_lhs.semantic_eq(name_rhs, scope)
            && temporary_lhs == temporary_rhs
            && unlogged_lhs == unlogged_rhs
            && table_lhs == table_rhs
    }
}

impl<'ast> SemanticEq<'ast> for LateralView {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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

        lateral_view_lhs.semantic_eq(lateral_view_rhs, scope)
            && lateral_view_name_lhs.semantic_eq(lateral_view_name_rhs, scope)
            && lateral_col_alias_lhs.semantic_eq(lateral_col_alias_rhs, scope)
            && outer_lhs == outer_rhs
    }
}

impl<'ast> SemanticEq<'ast> for GroupByExpr {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (
                GroupByExpr::All(group_by_with_modifiers_lhs),
                GroupByExpr::All(group_by_with_modifiers_rhs),
            ) => group_by_with_modifiers_lhs == group_by_with_modifiers_rhs,
            (
                GroupByExpr::Expressions(exprs_lhs, group_by_with_modifiers_lhs),
                GroupByExpr::Expressions(exprs_rhs, group_by_with_modifiers_rhs),
            ) => {
                exprs_lhs.semantic_eq(exprs_rhs, scope)
                    && group_by_with_modifiers_lhs == group_by_with_modifiers_rhs
            }
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for TableWithJoins {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self {
            relation: relation_lhs,
            joins: joins_lhs,
        } = self;
        let Self {
            relation: relation_rhs,
            joins: joins_rhs,
        } = other;

        relation_lhs.semantic_eq(relation_rhs, scope) && joins_lhs.semantic_eq(joins_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for Join {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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

        relation_lhs.semantic_eq(relation_rhs, scope)
            && global_lhs == global_rhs
            && join_operator_lhs.semantic_eq(join_operator_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for JoinOperator {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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
            ) => join_constraint_lhs.semantic_eq(join_constraint_rhs, scope),
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
                match_condition_lhs.semantic_eq(match_condition_rhs, scope)
                    && constraint_lhs.semantic_eq(constraint_rhs, scope)
            }
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for JoinConstraint {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (JoinConstraint::On(expr_lhs), JoinConstraint::On(expr_rhs)) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
            }
            (JoinConstraint::Using(idents_lhs), JoinConstraint::Using(idents_rhs)) => {
                idents_lhs.semantic_eq(idents_rhs, scope)
            }
            (JoinConstraint::Natural, JoinConstraint::Natural) => todo!(),
            (JoinConstraint::None, JoinConstraint::None) => todo!(),
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for TableFunctionArgs {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self {
            args: args_lhs,
            settings: settings_lhs,
        } = self;
        let Self {
            args: args_rhs,
            settings: settings_rhs,
        } = other;

        args_lhs.semantic_eq(args_rhs, scope) && settings_lhs.semantic_eq(settings_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for FunctionArg {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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
                name_lhs.semantic_eq(name_rhs, scope)
                    && arg_lhs.semantic_eq(arg_rhs, scope)
                    && operator_lhs == operator_rhs
            }
            (
                FunctionArg::Unnamed(function_arg_expr_lhs),
                FunctionArg::Unnamed(function_arg_expr_rhs),
            ) => function_arg_expr_lhs.semantic_eq(function_arg_expr_rhs, scope),
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for FunctionArgExpr {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (FunctionArgExpr::Expr(expr_lhs), FunctionArgExpr::Expr(expr_rhs)) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
            }
            (
                FunctionArgExpr::QualifiedWildcard(object_name_lhs),
                FunctionArgExpr::QualifiedWildcard(object_name_rhs),
            ) => object_name_lhs.semantic_eq(object_name_rhs, scope),
            (FunctionArgExpr::Wildcard, FunctionArgExpr::Wildcard) => true,
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for TableFactor {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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
                name_lhs.semantic_eq(name_rhs, scope)
                    && alias_lhs.semantic_eq(alias_rhs, scope)
                    && args_lhs.semantic_eq(args_rhs, scope)
                    && with_hints_lhs.semantic_eq(with_hints_rhs, scope)
                    && version_lhs == version_rhs
                    && with_ordinality_lhs == with_ordinality_rhs
                    && partitions_lhs.semantic_eq(partitions_rhs, scope)
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
                    && subquery_lhs.semantic_eq(subquery_rhs, scope)
                    && alias_lhs.semantic_eq(alias_rhs, scope)
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
            ) => expr_lhs.semantic_eq(expr_rhs, scope) && alias_lhs.semantic_eq(alias_rhs, scope),
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
                lateral_lhs == lateral_rhs
                    && name_rhs.semantic_eq(name_lhs, scope)
                    && args_rhs.semantic_eq(args_lhs, scope)
                    && alias_lhs.semantic_eq(alias_rhs, scope)
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
                alias_lhs.semantic_eq(alias_rhs, scope)
                    && array_exprs_lhs.semantic_eq(array_exprs_rhs, scope)
                    && with_offset_lhs == with_offset_rhs
                    && with_offset_alias_lhs.semantic_eq(with_offset_alias_rhs, scope)
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
                json_expr_lhs.semantic_eq(json_expr_rhs, scope)
                    && json_path_lhs == json_path_rhs
                    && columns_lhs.semantic_eq(columns_rhs, scope)
                    && alias_lhs.semantic_eq(alias_rhs, scope)
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
                table_with_joins_lhs.semantic_eq(table_with_joins_rhs, scope)
                    && alias_lhs.semantic_eq(alias_rhs, scope)
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
                table_lhs.semantic_eq(table_rhs, scope)
                    && aggregate_functions_lhs.semantic_eq(aggregate_functions_rhs, scope)
                    && value_column_lhs.semantic_eq(value_column_rhs, scope)
                    && value_source_lhs.semantic_eq(value_source_rhs, scope)
                    && default_on_null_lhs.semantic_eq(default_on_null_rhs, scope)
                    && alias_lhs.semantic_eq(alias_rhs, scope)
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
                table_lhs.semantic_eq(table_rhs, scope)
                    && value_lhs.semantic_eq(value_rhs, scope)
                    && name_lhs.semantic_eq(name_rhs, scope)
                    && columns_lhs.semantic_eq(columns_rhs, scope)
                    && alias_lhs.semantic_eq(alias_rhs, scope)
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
                table_lhs.semantic_eq(table_rhs, scope)
                    && partition_by_lhs.semantic_eq(partition_by_rhs, scope)
                    && order_by_lhs.semantic_eq(order_by_rhs, scope)
                    && measures_lhs.semantic_eq(measures_rhs, scope)
                    && rows_per_match_lhs == rows_per_match_rhs
                    && after_match_skip_lhs.semantic_eq(after_match_skip_rhs, scope)
                    && pattern_lhs.semantic_eq(pattern_rhs, scope)
                    && symbols_lhs.semantic_eq(symbols_rhs, scope)
                    && alias_lhs.semantic_eq(alias_rhs, scope)
            }
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for SymbolDefinition {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self {
            symbol: symbol_lhs,
            definition: defininition_lhs,
        } = self;
        let Self {
            symbol: symbol_rhs,
            definition: defininition_rhs,
        } = other;

        symbol_lhs.semantic_eq(symbol_rhs, scope)
            && defininition_lhs.semantic_eq(defininition_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for JsonTableNamedColumn {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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

        name_lhs.semantic_eq(name_rhs, scope)
            && r#type_lhs == r#type_rhs
            && path_lhs == path_rhs
            && exists_lhs == exists_rhs
            && on_empty_lhs == on_empty_rhs
            && on_error_lhs == on_error_rhs
    }
}

impl<'ast> SemanticEq<'ast> for MatchRecognizePattern {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (
                MatchRecognizePattern::Symbol(match_recognize_symbol_lhs),
                MatchRecognizePattern::Symbol(match_recognize_symbol_rhs),
            )
            | (
                MatchRecognizePattern::Exclude(match_recognize_symbol_lhs),
                MatchRecognizePattern::Exclude(match_recognize_symbol_rhs),
            ) => match_recognize_symbol_lhs.semantic_eq(match_recognize_symbol_rhs, scope),
            (
                MatchRecognizePattern::Permute(match_recognize_symbols_lhs),
                MatchRecognizePattern::Permute(match_recognize_symbols_rhs),
            ) => match_recognize_symbols_lhs.semantic_eq(match_recognize_symbols_rhs, scope),
            (
                MatchRecognizePattern::Concat(match_recognize_patterns_lhs),
                MatchRecognizePattern::Concat(match_recognize_patterns_rhs),
            ) => match_recognize_patterns_lhs.semantic_eq(match_recognize_patterns_rhs, scope),
            (
                MatchRecognizePattern::Group(match_recognize_pattern_lhs),
                MatchRecognizePattern::Group(match_recognize_pattern_rhs),
            ) => match_recognize_pattern_lhs.semantic_eq(match_recognize_pattern_rhs, scope),
            (
                MatchRecognizePattern::Alternation(match_recognize_patterns_lhs),
                MatchRecognizePattern::Alternation(match_recognize_patterns_rhs),
            ) => match_recognize_patterns_lhs.semantic_eq(match_recognize_patterns_rhs, scope),
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
                match_recognize_pattern_lhs.semantic_eq(match_recognize_pattern_rhs, scope)
                    && repetition_quantifier_lhs == repetition_quantifier_rhs
            }
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for MatchRecognizeSymbol {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (MatchRecognizeSymbol::Named(ident_lhs), MatchRecognizeSymbol::Named(ident_rhs)) => {
                ident_lhs.semantic_eq(ident_rhs, scope)
            }
            (MatchRecognizeSymbol::Start, MatchRecognizeSymbol::Start)
            | (MatchRecognizeSymbol::End, MatchRecognizeSymbol::End) => true,
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for AfterMatchSkip {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (AfterMatchSkip::PastLastRow, AfterMatchSkip::PastLastRow)
            | (AfterMatchSkip::ToNextRow, AfterMatchSkip::ToNextRow) => true,
            (AfterMatchSkip::ToFirst(ident_lhs), AfterMatchSkip::ToFirst(ident_rhs))
            | (AfterMatchSkip::ToLast(ident_lhs), AfterMatchSkip::ToLast(ident_rhs)) => {
                ident_lhs.semantic_eq(ident_rhs, scope)
            }
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for Measure {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self {
            expr: expr_lhs,
            alias: alias_lhs,
        } = self;
        let Self {
            expr: expr_rhs,
            alias: alias_rhs,
        } = other;

        expr_lhs.semantic_eq(expr_rhs, scope) && alias_lhs.semantic_eq(alias_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for PivotValueSource {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (PivotValueSource::List(items_lhs), PivotValueSource::List(items_rhs)) => {
                items_lhs.semantic_eq(items_rhs, scope)
            }
            (
                PivotValueSource::Any(order_by_exprs_lhs),
                PivotValueSource::Any(order_by_exprs_rhs),
            ) => order_by_exprs_lhs.semantic_eq(order_by_exprs_rhs, scope),
            (PivotValueSource::Subquery(query_lhs), PivotValueSource::Subquery(query_rhs)) => {
                query_lhs.semantic_eq(query_rhs, scope)
            }
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for ExprWithAlias {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self {
            expr: expr_lhs,
            alias: alias_lhs,
        } = self;
        let Self {
            expr: expr_rhs,
            alias: alias_rhs,
        } = other;

        expr_lhs.semantic_eq(expr_rhs, scope) && alias_lhs.semantic_eq(alias_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for JsonTableColumn {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (
                JsonTableColumn::Named(json_table_named_column_lhs),
                JsonTableColumn::Named(json_table_named_column_rhs),
            ) => json_table_named_column_lhs.semantic_eq(json_table_named_column_rhs, scope),
            (
                JsonTableColumn::ForOrdinality(ident_lhs),
                JsonTableColumn::ForOrdinality(ident_rhs),
            ) => ident_lhs.semantic_eq(ident_rhs, scope),
            (
                JsonTableColumn::Nested(json_table_nested_column_lhs),
                JsonTableColumn::Nested(json_table_nested_column_rhs),
            ) => json_table_nested_column_lhs.semantic_eq(json_table_nested_column_rhs, scope),
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for JsonTableNestedColumn {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self {
            path: path_lhs,
            columns: columns_lhs,
        } = self;
        let Self {
            path: path_rhs,
            columns: columns_rhs,
        } = other;

        path_lhs == path_rhs && columns_lhs.semantic_eq(columns_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for Distinct {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (Distinct::Distinct, Distinct::Distinct) => true,
            (Distinct::On(exprs_lhs), Distinct::On(exprs_rhs)) => {
                exprs_lhs.semantic_eq(exprs_rhs, scope)
            }
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for Top {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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
            && quantity_lhs.semantic_eq(quantity_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for SelectItem {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (SelectItem::UnnamedExpr(expr_lhs), SelectItem::UnnamedExpr(expr_rhs)) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
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
            ) => expr_lhs.semantic_eq(expr_rhs, scope) && alias_lhs.semantic_eq(alias_rhs, scope),
            (
                SelectItem::QualifiedWildcard(object_name_lhs, wildcard_additional_options_lhs),
                SelectItem::QualifiedWildcard(object_name_rhs, wildcard_additional_options_rhs),
            ) => {
                object_name_lhs.semantic_eq(object_name_rhs, scope)
                    && wildcard_additional_options_lhs
                        .semantic_eq(wildcard_additional_options_rhs, scope)
            }
            (
                SelectItem::Wildcard(wildcard_additional_options_lhs),
                SelectItem::Wildcard(wildcard_additional_options_rhs),
            ) => {
                wildcard_additional_options_lhs.semantic_eq(wildcard_additional_options_rhs, scope)
            }
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for WildcardAdditionalOptions {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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
            && opt_exclude_lhs.semantic_eq(opt_exclude_rhs, scope)
            && opt_except_lhs.semantic_eq(opt_except_rhs, scope)
            && opt_replace_lhs.semantic_eq(opt_replace_rhs, scope)
            && opt_rename_lhs.semantic_eq(opt_rename_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for RenameSelectItem {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (
                RenameSelectItem::Single(ident_with_alias_lhs),
                RenameSelectItem::Single(ident_with_alias_rhs),
            ) => ident_with_alias_lhs.semantic_eq(ident_with_alias_rhs, scope),
            (RenameSelectItem::Multiple(items_lhs), RenameSelectItem::Multiple(items_rhs)) => {
                items_lhs.semantic_eq(items_rhs, scope)
            }
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for IdentWithAlias {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self {
            ident: ident_lhs,
            alias: alias_lhs,
        } = self;
        let Self {
            ident: ident_rhs,
            alias: alias_rhs,
        } = other;

        ident_lhs.semantic_eq(ident_rhs, scope) && alias_lhs.semantic_eq(alias_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for ExcludeSelectItem {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (ExcludeSelectItem::Single(ident_lhs), ExcludeSelectItem::Single(ident_rhs)) => {
                ident_lhs.semantic_eq(ident_rhs, scope)
            }
            (ExcludeSelectItem::Multiple(idents_lhs), ExcludeSelectItem::Multiple(idents_rhs)) => {
                idents_lhs.semantic_eq(idents_rhs, scope)
            }
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for ExceptSelectItem {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self {
            first_element: first_element_lhs,
            additional_elements: additional_elements_lhs,
        } = self;
        let Self {
            first_element: first_element_rhs,
            additional_elements: additional_elements_rhs,
        } = other;

        first_element_lhs.semantic_eq(first_element_rhs, scope)
            && additional_elements_lhs.semantic_eq(additional_elements_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for TopQuantity {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (TopQuantity::Expr(expr_lhs), TopQuantity::Expr(expr_rhs)) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
            }
            (TopQuantity::Constant(c_lhs), TopQuantity::Constant(c_rhs)) => c_lhs == c_rhs,
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for ReplaceSelectItem {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self { items: items_lhs } = self;
        let Self { items: items_rhs } = other;

        items_lhs.semantic_eq(items_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for ReplaceSelectElement {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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

        expr_lhs.semantic_eq(expr_rhs, scope)
            && column_name_lhs.semantic_eq(column_name_rhs, scope)
            && as_keyword_lhs == as_keyword_rhs
    }
}

impl<'ast> SemanticEq<'ast> for DateTimeField {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (Self::Week(ident_lhs), Self::Week(ident_rhs)) => {
                ident_lhs.semantic_eq(ident_rhs, scope)
            }
            (Self::Custom(ident_lhs), Self::Custom(ident_rhs)) => {
                ident_lhs.semantic_eq(ident_rhs, scope)
            }
            _ => mem::discriminant(self) == mem::discriminant(other),
        }
    }
}

impl<'ast> SemanticEq<'ast> for CeilFloorKind {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (
                CeilFloorKind::DateTimeField(date_time_field_lhs),
                CeilFloorKind::DateTimeField(date_time_field_rhs),
            ) => date_time_field_lhs.semantic_eq(date_time_field_rhs, scope),
            (CeilFloorKind::Scale(value_lhs), CeilFloorKind::Scale(value_rhs)) => {
                value_lhs == value_rhs
            }
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for StructField {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self {
            field_name: field_name_lhs,
            field_type: field_type_lhs,
        } = self;
        let Self {
            field_name: field_name_rhs,
            field_type: field_type_rhs,
        } = other;

        field_name_lhs.semantic_eq(field_name_rhs, scope) && field_type_lhs == field_type_rhs
    }
}

impl<'ast> SemanticEq<'ast> for DictionaryField {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self {
            key: key_lhs,
            value: value_lhs,
        } = self;
        let Self {
            key: key_rhs,
            value: value_rhs,
        } = other;

        key_lhs.semantic_eq(key_rhs, scope) && value_lhs.semantic_eq(value_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for Map {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self {
            entries: entries_lhs,
        } = self;
        let Self {
            entries: entries_rhs,
        } = other;

        entries_lhs.semantic_eq(entries_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for MapEntry {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self {
            key: key_lhs,
            value: value_lhs,
        } = self;
        let Self {
            key: key_rhs,
            value: value_rhs,
        } = other;

        key_lhs.semantic_eq(key_rhs, scope) && value_lhs.semantic_eq(value_rhs, scope)
    }
}

impl<'ast> SemanticEq<'ast> for Subscript {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (Subscript::Index { index: index_lhs }, Subscript::Index { index: index_rhs }) => {
                index_lhs.semantic_eq(index_rhs, scope)
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
                lower_bound_lhs.semantic_eq(lower_bound_rhs, scope)
                    && upper_bound_lhs.semantic_eq(upper_bound_rhs, scope)
                    && stride_lhs.semantic_eq(stride_rhs, scope)
            }
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for Array {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self {
            elem: elem_lhs,
            named: named_lhs,
        } = self;
        let Self {
            elem: elem_rhs,
            named: named_rhs,
        } = other;

        elem_lhs.semantic_eq(elem_rhs, scope) && named_lhs == named_rhs
    }
}

impl<'ast> SemanticEq<'ast> for Interval {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
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

        value_lhs.semantic_eq(value_rhs, scope)
            && leading_field_lhs.semantic_eq(leading_field_rhs, scope)
            && leading_precision_lhs == leading_precision_rhs
            && last_field_lhs.semantic_eq(last_field_rhs, scope)
            && fractional_seconds_precision_lhs == fractional_seconds_precision_rhs
    }
}

impl<'ast> SemanticEq<'ast> for LambdaFunction {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        let Self {
            params: params_lhs,
            body: body_lhs,
        } = self;
        let Self {
            params: params_rhs,
            body: body_rhs,
        } = other;

        params_lhs.semantic_eq(params_rhs, scope) && body_lhs.semantic_eq(body_rhs, scope)
    }
}

impl<'ast, T: SemanticEq<'ast>> SemanticEq<'ast> for OneOrManyWithParens<T> {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        match (self, other) {
            (OneOrManyWithParens::One(item_lhs), OneOrManyWithParens::One(item_rhs)) => {
                item_lhs.semantic_eq(item_rhs, scope)
            }
            (OneOrManyWithParens::Many(items_lhs), OneOrManyWithParens::Many(items_rhs)) => {
                items_lhs.semantic_eq(items_rhs, scope)
            }
            _ => false,
        }
    }
}

impl<'ast> SemanticEq<'ast> for Expr {
    fn semantic_eq(&self, other: &Self, scope: &ScopeTracker<'ast>) -> bool {
        // Expr::Nested(_) requires special handling because the parens are superfluous when it comes to equality.
        match (self, other) {
            (Expr::Nested(expr_lhs), expr_rhs) => {
                return (**expr_lhs).semantic_eq(expr_rhs, scope)
            }
            (expr_lhs, Expr::Nested(expr_rhs)) => return expr_lhs.semantic_eq(expr_rhs, scope),
            _ => {}
        }

        // If the discriminants are different then the nodes are different, so we can bail out.
        if mem::discriminant(self) != mem::discriminant(other) {
            return false;
        }

        match (self, other) {
            (Expr::Identifier(ident_lhs), Expr::Identifier(ident_rhs)) => scope
                .resolve_ident(ident_lhs)
                .is_ok_and(|_| scope.resolve_ident(ident_rhs).is_ok()),
            (Expr::CompoundIdentifier(idents_lhs), Expr::CompoundIdentifier(idents_rhs)) => scope
                .resolve_compound_ident(idents_lhs)
                .is_ok_and(|_| {
                    scope
                        .resolve_compound_ident(idents_rhs)
                        .is_ok()
                }),
            (
                Expr::JsonAccess {
                    value: value_lhs,
                    path: path_lhs,
                },
                Expr::JsonAccess {
                    value: value_rhs,
                    path: path_rhs,
                },
            ) => value_lhs.semantic_eq(value_rhs, scope) && path_lhs.semantic_eq(path_rhs, scope),
            (
                Expr::CompositeAccess {
                    expr: expr_lhs,
                    key: key_lhs,
                },
                Expr::CompositeAccess {
                    expr: expr_rhs,
                    key: key_rhs,
                },
            ) => expr_lhs.semantic_eq(expr_rhs, scope) && key_lhs.semantic_eq(key_rhs, scope),
            (Expr::IsFalse(expr_lhs), Expr::IsFalse(expr_rhs)) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
            }
            (Expr::IsNotFalse(expr_lhs), Expr::IsNotFalse(expr_rhs)) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
            }
            (Expr::IsTrue(expr_lhs), Expr::IsTrue(expr_rhs)) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
            }
            (Expr::IsNotTrue(expr_lhs), Expr::IsNotTrue(expr_rhs)) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
            }
            (Expr::IsNull(expr_lhs), Expr::IsNull(expr_rhs)) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
            }
            (Expr::IsNotNull(expr_lhs), Expr::IsNotNull(expr_rhs)) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
            }
            (Expr::IsUnknown(expr_lhs), Expr::IsUnknown(expr_rhs)) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
            }
            (Expr::IsNotUnknown(expr_lhs), Expr::IsNotUnknown(expr_rhs)) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
            }
            (
                Expr::IsDistinctFrom(expr_lhs, expr1_lhs),
                Expr::IsDistinctFrom(expr_rhs, expr1_rhs),
            ) => expr_lhs.semantic_eq(expr_rhs, scope) && expr1_lhs.semantic_eq(expr1_rhs, scope),
            (
                Expr::IsNotDistinctFrom(expr_lhs, expr1_lhs),
                Expr::IsNotDistinctFrom(expr_rhs, expr1_rhs),
            ) => expr_lhs.semantic_eq(expr_rhs, scope) && expr1_lhs.semantic_eq(expr1_rhs, scope),
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
                expr_lhs.semantic_eq(expr_rhs, scope)
                    && list_lhs
                        .iter()
                        .zip(list_rhs.iter())
                        .all(|(l, r)| l.semantic_eq(r, scope))
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
                expr_lhs.semantic_eq(expr_rhs, scope)
                    && subquery_lhs.semantic_eq(subquery_rhs, scope)
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
                expr_lhs.semantic_eq(expr_rhs, scope)
                    && array_expr_lhs.semantic_eq(array_expr_rhs, scope)
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
                expr_lhs.semantic_eq(expr_rhs, scope)
                    && negated_lhs == negated_rhs
                    && low_lhs.semantic_eq(low_rhs, scope)
                    && high_lhs.semantic_eq(high_rhs, scope)
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
                left_lhs.semantic_eq(left_rhs, scope)
                    && right_lhs.semantic_eq(right_rhs, scope)
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
                    && expr_lhs.semantic_eq(expr_rhs, scope)
                    && pattern_lhs.semantic_eq(pattern_rhs, scope)
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
                    && expr_lhs.semantic_eq(expr_rhs, scope)
                    && pattern_lhs.semantic_eq(pattern_rhs, scope)
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
                    && expr_lhs.semantic_eq(expr_rhs, scope)
                    && pattern_lhs.semantic_eq(pattern_rhs, scope)
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
                    && expr_lhs.semantic_eq(expr_rhs, scope)
                    && pattern_lhs.semantic_eq(pattern_rhs, scope)
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
                left_lhs.semantic_eq(left_rhs, scope)
                    && compare_op_lhs == compare_op_rhs
                    && right_lhs.semantic_eq(right_rhs, scope)
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
                left_lhs.semantic_eq(left_rhs, scope)
                    && compare_op_lhs == compare_op_rhs
                    && right_lhs.semantic_eq(right_rhs, scope)
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
            ) => op_lhs == op_rhs && expr_lhs.semantic_eq(expr_rhs, scope),
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
                    && expr_lhs.semantic_eq(expr_rhs, scope)
                    && data_type_lhs == data_type_rhs
                    && charset_lhs.semantic_eq(charset_rhs, scope)
                    && target_before_value_lhs == target_before_value_rhs
                    && styles_lhs.semantic_eq(styles_rhs, scope)
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
                    && expr_lhs.semantic_eq(expr_rhs, scope)
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
                timestamp_lhs.semantic_eq(timestamp_rhs, scope)
                    && time_zone_lhs.semantic_eq(time_zone_rhs, scope)
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
                field_lhs.semantic_eq(field_rhs, scope)
                    && syntax_lhs == syntax_rhs
                    && expr_lhs.semantic_eq(expr_rhs, scope)
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
            ) => expr_lhs.semantic_eq(expr_rhs, scope) && field_lhs.semantic_eq(field_rhs, scope),
            (
                Expr::Floor {
                    expr: expr_lhs,
                    field: field_lhs,
                },
                Expr::Floor {
                    expr: expr_rhs,
                    field: field_rhs,
                },
            ) => expr_lhs.semantic_eq(expr_rhs, scope) && field_lhs.semantic_eq(field_rhs, scope),
            (
                Expr::Position {
                    expr: expr_lhs,
                    r#in: in_lhs,
                },
                Expr::Position {
                    expr: expr_rhs,
                    r#in: in_rhs,
                },
            ) => expr_lhs.semantic_eq(expr_rhs, scope) && in_lhs.semantic_eq(in_rhs, scope),
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
                expr_lhs.semantic_eq(expr_rhs, scope)
                    && substring_from_lhs.semantic_eq(substring_from_rhs, scope)
                    && substring_for_lhs.semantic_eq(substring_for_rhs, scope)
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
                expr_lhs.semantic_eq(expr_rhs, scope)
                    && trim_where_lhs == trim_where_rhs
                    && trim_what_lhs.semantic_eq(trim_what_rhs, scope)
                    && trim_characters_lhs.semantic_eq(trim_characters_rhs, scope)
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
                expr_lhs.semantic_eq(expr_rhs, scope)
                    && overlay_what_lhs.semantic_eq(overlay_what_rhs, scope)
                    && overlay_from_lhs.semantic_eq(overlay_from_rhs, scope)
                    && overlay_for_lhs.semantic_eq(overlay_for_rhs, scope)
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
            ) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
                    && collation_lhs.semantic_eq(collation_rhs, scope)
            }
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
            ) => column_lhs.semantic_eq(column_rhs, scope) && keys_lhs.semantic_eq(keys_rhs, scope),
            (Expr::Function(function_lhs), Expr::Function(function_rhs)) => {
                function_lhs.semantic_eq(function_rhs, scope)
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
                operand_lhs.semantic_eq(operand_rhs, scope)
                    && conditions_lhs.semantic_eq(conditions_rhs, scope)
                    && results_lhs.semantic_eq(results_rhs, scope)
                    && else_result_lhs.semantic_eq(else_result_rhs, scope)
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
            ) => subquery_lhs.semantic_eq(subquery_rhs, scope) && negated_lhs == negated_rhs,
            (Expr::Subquery(query_lhs), Expr::Subquery(query_rhs)) => {
                query_lhs.semantic_eq(query_rhs, scope)
            }
            (Expr::GroupingSets(items_lhs), Expr::GroupingSets(items_rhs)) => {
                items_lhs.semantic_eq(items_rhs, scope)
            }
            (Expr::Cube(items_lhs), Expr::Cube(items_rhs)) => {
                items_lhs.semantic_eq(items_rhs, scope)
            }
            (Expr::Rollup(items_lhs), Expr::Rollup(items_rhs)) => {
                items_lhs.semantic_eq(items_rhs, scope)
            }
            (Expr::Tuple(exprs_lhs), Expr::Tuple(exprs_rhs)) => {
                exprs_lhs.semantic_eq(exprs_rhs, scope)
            }
            (
                Expr::Struct {
                    values: values_lhs,
                    fields: fields_lhs,
                },
                Expr::Struct {
                    values: values_rhs,
                    fields: fields_rhs,
                },
            ) => {
                values_lhs.semantic_eq(values_rhs, scope)
                    && fields_lhs.semantic_eq(fields_rhs, scope)
            }
            (
                Expr::Named {
                    expr: expr_lhs,
                    name: name_lhs,
                },
                Expr::Named {
                    expr: expr_rhs,
                    name: name_rhs,
                },
            ) => expr_lhs.semantic_eq(expr_rhs, scope) && name_lhs.semantic_eq(name_rhs, scope),
            (Expr::Dictionary(dictionary_fields_lhs), Expr::Dictionary(dictionary_fields_rhs)) => {
                dictionary_fields_lhs.semantic_eq(dictionary_fields_rhs, scope)
            }
            (Expr::Map(map_lhs), Expr::Map(map_rhs)) => map_lhs.semantic_eq(map_rhs, scope),
            (
                Expr::Subscript {
                    expr: expr_lhs,
                    subscript: subscript_lhs,
                },
                Expr::Subscript {
                    expr: expr_rhs,
                    subscript: subscript_rhs,
                },
            ) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
                    && subscript_lhs.semantic_eq(subscript_rhs, scope)
            }
            (Expr::Array(array_lhs), Expr::Array(array_rhs)) => {
                array_lhs.semantic_eq(array_rhs, scope)
            }
            (Expr::Interval(interval_lhs), Expr::Interval(interval_rhs)) => {
                interval_lhs.semantic_eq(interval_rhs, scope)
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
                columns_lhs.semantic_eq(columns_rhs, scope)
                    && match_value_lhs == match_value_rhs
                    && opt_search_modifier_lhs == opt_search_modifier_rhs
            }
            (Expr::Wildcard, Expr::Wildcard) => true,
            (
                Expr::QualifiedWildcard(object_name_lhs),
                Expr::QualifiedWildcard(object_name_rhs),
            ) => object_name_lhs.semantic_eq(object_name_rhs, scope),
            (Expr::OuterJoin(expr_lhs), Expr::OuterJoin(expr_rhs)) => {
                expr_lhs.semantic_eq(expr_rhs, scope)
            }
            (Expr::Prior(expr_lhs), Expr::Prior(expr_rhs)) => expr_lhs.semantic_eq(expr_rhs, scope),
            (Expr::Lambda(lambda_function_lhs), Expr::Lambda(lambda_function_rhs)) => {
                lambda_function_lhs.semantic_eq(lambda_function_rhs, scope)
            }
            _ => false,
        }
    }
}
