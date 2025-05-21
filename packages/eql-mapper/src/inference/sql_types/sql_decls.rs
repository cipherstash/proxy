use std::{collections::HashMap, sync::LazyLock};

use sqltk::parser::ast::BinaryOperator;

use crate::{
    binop, sql_fn,
    unifier::{EqlTrait, TraitBound, TypeArg},
};

use super::{
    CompoundIdent, ExplicitBinaryOpRule, ExplicitSqlFunctionRule, SqlBinaryOp, SqlFunction,
};

/// SQL operators that can accept EQL types.
///
/// Rule syntax: `($lhs_type $op $rhs_type) -> $return_type { where $bounds }?`
static SQL_BINARY_OPERATORS: LazyLock<HashMap<BinaryOperator, ExplicitBinaryOpRule>> =
    LazyLock::new(|| {
        // Fun Fact™️: the SQL operators that also happen to be Rust operators (consisting of a single lexical token) do
        // not have to be wrapped in parens.  Operators that are composed of multiple Rust tokens must be wrapped in
        // parens in order to make `macro_rules` happy.
        vec![
            binop!( (T = T) -> Native where T: Eq ),
            binop!( (T (<>) T) -> Native where T: Eq ),
            binop!( (T <= T) -> Native where T: Ord ),
            binop!( (T >= T) -> Native where T: Ord ),
            binop!( (T < T) -> Native where T: Ord ),
            binop!( (T > T) -> Native where T: Ord ),
            binop!( (J (->) A) -> J where J: Json, A: JsonAccessor<J> ),
            binop!( (J (->>) A) -> J where J: Json, A: JsonAccessor<J> ),
            binop!( (J (@>) A) -> Native where J: Json ),
            binop!( (J (<@) A) -> Native where J: Json ),
            binop!( (J (@?) A) -> Native where J: Json ),
        ]
        .into_iter()
        .collect()
    });

pub(crate) fn get_sql_binop_rule(op: &BinaryOperator) -> SqlBinaryOp {
    SQL_BINARY_OPERATORS
        .get(op)
        .map(SqlBinaryOp::Explicit)
        .unwrap_or(SqlBinaryOp::Fallback)
}

/// SQL functions that are handled with special case type checking rules for EQL.
static SQL_FUNCTION_TYPES: LazyLock<HashMap<CompoundIdent, ExplicitSqlFunctionRule>> =
    LazyLock::new(|| {
        // # SQL function declations.
        //
        // A single uppercase lettter such as `T` or `U` denotes a type variable. During type unification a type
        // variable must resolve to the same type at every location it is used (just like in Rust).
        //
        // `Native` literally means `Type::Constructor(Constructor::Value(Value::Native(_)))`. From the perspective of
        // the EQL Mapper `Native` is a concrete type.
        //
        // Type variables can resolve to an EQL type (e.g. `Type::Constructor(Constructor::Value(Value::Eql(_)))` )`OR
        // `Native`.
        //
        // `Native` automatically satisfies *all* trait bounds. This is the trick that keeps the complexity of EQL
        // Mapper's type system small enough to be tractable for a small team of engineers. It is a *safe* strategy
        // because even though EQL Mapper will not catch a type error, Postgres will.
        //
        // The Postgres versions of `count`, `min`, `max` etc are defined in the `pg_catalog` namespace. `pg_catalog` is
        // prepended to the `search_path` by Postgres. When resolving the names of registered unqualified functions in
        // this list, `pg_catalog`  is assumed to be the schema. Additionally, functions in `pg_catalog` will be
        // rewritten to their EQL counterpart by the EQL Mapper.
        let sql_fns = vec![
            sql_fn!(pg_catalog.count(T) -> Native),
            sql_fn!(pg_catalog.min(T) -> T where T: Ord),
            sql_fn!(pg_catalog.max(T) -> T where T: Ord),
            sql_fn!(pg_catalog.jsonb_path_query(T, U) -> T where T: Json, U: JsonAccessor<T>),
            sql_fn!(pg_catalog.jsonb_path_query_first(T, U) -> T where T: Json, U: JsonAccessor<T>),
            sql_fn!(pg_catalog.jsonb_path_exists(T, U) -> Native where T: Json, U: JsonAccessor<T>),
            sql_fn!(pg_catalog.jsonb_array_length(T) -> Native where T: Json),
            sql_fn!(pg_catalog.jsonb_array_elements(T) -> T where T: Json),
            sql_fn!(pg_catalog.jsonb_array_elements_text(T) -> T where T: Json),
            sql_fn!(eql_v1.min(T) -> T where T: Ord),
            sql_fn!(eql_v1.max(T) -> T where T: Ord),
            sql_fn!(eql_v1.jsonb_path_query(T, U) -> T where T: Json, U: JsonAccessor<T>),
            sql_fn!(eql_v1.jsonb_path_query_first(T, U) -> T where T: Json, U: JsonAccessor<T>),
            sql_fn!(eql_v1.jsonb_path_exists(T, U) -> Native where T: Json, U: JsonAccessor<T>),
            sql_fn!(eql_v1.jsonb_array_length(T) -> Native where T: Json),
            sql_fn!(eql_v1.jsonb_array_elements(T) -> T where T: Json),
            sql_fn!(eql_v1.jsonb_array_elements_text(T) -> T where T: Json),
        ];

        HashMap::from_iter(sql_fns.into_iter().map(|rule| (rule.name.clone(), rule)))
    });

pub(crate) fn get_sql_function(fn_name: &CompoundIdent) -> SqlFunction {
    SQL_FUNCTION_TYPES
        .get(fn_name)
        .map(SqlFunction::Explicit)
        .unwrap_or(SqlFunction::Fallback)
}
