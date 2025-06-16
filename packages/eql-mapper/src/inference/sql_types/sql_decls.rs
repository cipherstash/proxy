use std::{collections::HashMap, sync::LazyLock};

use eql_mapper_macros::{binary_operators, functions};
use sqltk::parser::ast::{BinaryOperator, Ident, ObjectName, ObjectNamePart};

use crate::unifier::{BinaryOpDecl, FunctionDecl};

use super::{SqlBinaryOp, SqlFunction};

/// SQL operators that can accept EQL types.
static SQL_BINARY_OPERATORS: LazyLock<HashMap<BinaryOperator, BinaryOpDecl>> =
    LazyLock::new(|| {
        let ops = binary_operators! {
            <T>(T = T) -> Native where T: Eq;
            <T>(T <> T) -> Native where T: Eq;
            <T>(T <= T) -> Native where T: Ord;
            <T>(T >= T) -> Native where T: Ord;
            <T>(T < T) -> Native where T: Ord;
            <T>(T > T) -> Native where T: Ord;
            <T>(T -> <T as JsonLike>::Accessor) -> T where T: JsonLike;
            <T>(T ->> <T as JsonLike>::Accessor) -> T where T: JsonLike;
            <T>(T @> T) -> Native where T: Contain;
            <T>(T <@ T) -> Native where T: Contain;
            <T>(T ~~ <T as TokenMatch>::Tokenized) -> Native where T: TokenMatch; // LIKE
            <T>(T !~~ <T as TokenMatch>::Tokenized) -> Native where T: TokenMatch; // NOT LIKE
            <T>(T ~~* <T as TokenMatch>::Tokenized) -> Native where T: TokenMatch; // ILIKE
            <T>(T !~~* <T as TokenMatch>::Tokenized) -> Native where T: TokenMatch; // NOT ILIKE
        };
        ops.into_iter()
            .map(|binary_op_spec| (binary_op_spec.op.clone(), binary_op_spec))
            .collect::<HashMap<_, _>>()
    });

pub(crate) fn get_sql_binop_rule(op: &BinaryOperator) -> SqlBinaryOp {
    SQL_BINARY_OPERATORS
        .get(op)
        .map(SqlBinaryOp::Explicit)
        .unwrap_or(SqlBinaryOp::Fallback)
}

/// SQL functions that are handled with special case type checking rules for EQL.
static SQL_FUNCTION_TYPES: LazyLock<HashMap<ObjectName, FunctionDecl>> = LazyLock::new(|| {
    // # SQL function declations.
    //
    // `Native` automatically satisfies *all* trait bounds. This is the trick that keeps the complexity of EQL
    // Mapper's type system small enough to be tractable for a small team of engineers. It is a *safe* strategy
    // because even though EQL Mapper will not catch a type error, Postgres will.
    //
    // The Postgres versions of `count`, `min`, `max` etc are defined in the `pg_catalog` namespace. `pg_catalog` is
    // prepended to the `search_path` by Postgres. When resolving the names of registered unqualified functions in
    // this list, `pg_catalog`  is assumed to be the schema. Additionally, functions in `pg_catalog` will be
    // rewritten to their EQL counterpart by the EQL Mapper.

    let items = functions! {
        pg_catalog.count<T>(T) -> Native;
        pg_catalog.min<T>(T) -> T where T: Ord;
        pg_catalog.max<T>(T) -> T where T: Ord;
        pg_catalog.jsonb_path_query<J>(J, <J as JsonLike>::Path) -> J where J: JsonLike;
        pg_catalog.jsonb_path_query_first<J>(J, <J as JsonLike>::Path) -> J where J: JsonLike;
        pg_catalog.jsonb_path_exists<J>(J, <J as JsonLike>::Path) -> Native where J: JsonLike;
        pg_catalog.jsonb_array_length<J>(J) -> Native where J: JsonLike;
        pg_catalog.jsonb_array_elements<J>(J) -> J where J: JsonLike;
        pg_catalog.jsonb_array_elements_text<J>(J) -> J where J: JsonLike;
        eql_v2.min<T>(T) -> T where T: Ord;
        eql_v2.max<T>(T) -> T where T: Ord;
        eql_v2.jsonb_path_query<J>(J, <J as JsonLike>::Path) -> J where J: JsonLike;
        eql_v2.jsonb_path_query_first<J>(J, <J as JsonLike>::Path) -> J where J: JsonLike;
        eql_v2.jsonb_path_exists<J>(J, <J as JsonLike>::Path) -> Native where J: JsonLike;
        eql_v2.jsonb_array_length<J>(J) -> Native where J: JsonLike;
        eql_v2.jsonb_array_elements<J>(J) -> J where J: JsonLike;
        eql_v2.jsonb_array_elements_text<J>(J) -> J where J: JsonLike;
    };

    HashMap::from_iter(
        items
            .into_iter()
            .map(|rule: FunctionDecl| (rule.name.clone(), rule)),
    )
});

pub(crate) fn get_sql_function(fn_name: &ObjectName) -> SqlFunction {
    // FIXME: this is a hack and we need proper schema resolution logic
    let fully_qualified_fn_name = if fn_name.0.len() == 1 {
        &ObjectName(vec![
            ObjectNamePart::Identifier(Ident::new("pg_catalog")),
            fn_name.0[0].clone(),
        ])
    } else {
        fn_name
    };

    SQL_FUNCTION_TYPES
        .get(fully_qualified_fn_name)
        .map(SqlFunction::Explicit)
        .unwrap_or(SqlFunction::Fallback)
}

#[cfg(test)]
mod tests {
    use crate::inference::sql_types::sql_decls::{SQL_BINARY_OPERATORS, SQL_FUNCTION_TYPES};

    #[test]
    fn binops_load_properly() {
        let _ = &*SQL_BINARY_OPERATORS;
    }

    #[test]
    fn sqlfns_load_properly() {
        let _ = &*SQL_FUNCTION_TYPES;
    }
}
