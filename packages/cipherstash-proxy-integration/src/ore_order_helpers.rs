#![allow(dead_code)]
//! Shared ORE ordering test helpers.
//!
//! Used by both `map_ore_index_order` (default keyset) and `multitenant::ore_order`
//! (per-tenant keysets) to avoid duplicating test logic.
//!
//! Each helper takes a `table` name so callers can target their own per-test
//! fixture table — this prevents parallel-test races on a shared `encrypted`
//! table.

use std::fmt::Debug;
use tokio_postgres::types::{FromSql, ToSql};
use tokio_postgres::SimpleQueryMessage;

use crate::common::{interleaved_indices, random_id};

/// Sort direction for ORE ordering tests.
#[derive(Clone, Copy)]
pub enum SortDirection {
    Asc,
    Desc,
}

impl SortDirection {
    pub fn as_sql(&self) -> &'static str {
        match self {
            SortDirection::Asc => "ASC",
            SortDirection::Desc => "DESC",
        }
    }
}

/// Text ASC ordering with lexicographic edge cases.
pub async fn ore_order_text(client: &tokio_postgres::Client, table: &str) {
    let values = [
        "aardvark",
        "aplomb",
        "apparatus",
        "chimera",
        "chrysalis",
        "chrysanthemum",
        "zephyr",
    ];

    let insert_sql = format!("INSERT INTO {table} (id, encrypted_text) VALUES ($1, $2)");

    for idx in interleaved_indices(values.len()) {
        client
            .query(&insert_sql, &[&random_id(), &values[idx]])
            .await
            .unwrap();
    }

    let sql = format!("SELECT encrypted_text FROM {table} ORDER BY encrypted_text");
    let rows = client.query(&sql, &[]).await.unwrap();

    let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<String>>();
    let expected: Vec<String> = values.iter().map(|s| s.to_string()).collect();

    assert_eq!(actual, expected);
}

/// Text DESC ordering with lexicographic edge cases.
pub async fn ore_order_text_desc(client: &tokio_postgres::Client, table: &str) {
    let values = [
        "aardvark",
        "aplomb",
        "apparatus",
        "chimera",
        "chrysalis",
        "chrysanthemum",
        "zephyr",
    ];

    let insert_sql = format!("INSERT INTO {table} (id, encrypted_text) VALUES ($1, $2)");

    for idx in interleaved_indices(values.len()) {
        client
            .query(&insert_sql, &[&random_id(), &values[idx]])
            .await
            .unwrap();
    }

    let sql = format!("SELECT encrypted_text FROM {table} ORDER BY encrypted_text DESC");
    let rows = client.query(&sql, &[]).await.unwrap();

    let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<String>>();
    let expected: Vec<String> = values.iter().rev().map(|s| s.to_string()).collect();

    assert_eq!(actual, expected);
}

/// NULLs sort last in ASC by default.
pub async fn ore_order_nulls_last_by_default(client: &tokio_postgres::Client, table: &str) {
    let s_one = "a";
    let s_two = "b";

    client
        .query(
            &format!("INSERT INTO {table} (id) values ($1)"),
            &[&random_id()],
        )
        .await
        .unwrap();

    let sql = format!(
        "
        INSERT INTO {table} (id, encrypted_text)
        VALUES ($1, $2), ($3, $4)
    "
    );

    client
        .query(&sql, &[&random_id(), &s_one, &random_id(), &s_two])
        .await
        .unwrap();

    let sql = format!("SELECT encrypted_text FROM {table} ORDER BY encrypted_text");
    let rows = client.query(&sql, &[]).await.unwrap();

    let actual = rows
        .iter()
        .map(|row| row.get(0))
        .collect::<Vec<Option<String>>>();
    let expected = vec![Some(s_one.to_string()), Some(s_two.to_string()), None];

    assert_eq!(actual, expected);
}

/// NULLS FIRST clause.
pub async fn ore_order_nulls_first(client: &tokio_postgres::Client, table: &str) {
    let s_one = "a";
    let s_two = "b";

    let sql = format!(
        "
        INSERT INTO {table} (id, encrypted_text)
        VALUES ($1, $2), ($3, $4)
    "
    );

    client
        .query(&sql, &[&random_id(), &s_one, &random_id(), &s_two])
        .await
        .unwrap();

    client
        .query(
            &format!("INSERT INTO {table} (id) values ($1)"),
            &[&random_id()],
        )
        .await
        .unwrap();

    let sql = format!(
        "SELECT encrypted_text FROM {table} ORDER BY encrypted_text NULLS FIRST"
    );
    let rows = client.query(&sql, &[]).await.unwrap();

    let actual = rows
        .iter()
        .map(|row| row.get(0))
        .collect::<Vec<Option<String>>>();
    let expected = vec![None, Some(s_one.to_string()), Some(s_two.to_string())];

    assert_eq!(actual, expected);
}

/// Fully qualified column name: `<table>.encrypted_text`.
pub async fn ore_order_qualified_column(client: &tokio_postgres::Client, table: &str) {
    let s_one = "a";
    let s_two = "b";
    let s_three = "c";

    let sql = format!(
        "
        INSERT INTO {table} (id, encrypted_text)
        VALUES ($1, $2), ($3, $4), ($5, $6)
    "
    );

    client
        .query(
            &sql,
            &[
                &random_id(),
                &s_two,
                &random_id(),
                &s_one,
                &random_id(),
                &s_three,
            ],
        )
        .await
        .unwrap();

    let sql = format!("SELECT encrypted_text FROM {table} ORDER BY {table}.encrypted_text");
    let rows = client.query(&sql, &[]).await.unwrap();

    let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<String>>();
    let expected = vec![s_one, s_two, s_three];

    assert_eq!(actual, expected);
}

/// Table alias: `e.encrypted_text`.
pub async fn ore_order_qualified_column_with_alias(client: &tokio_postgres::Client, table: &str) {
    let s_one = "a";
    let s_two = "b";
    let s_three = "c";

    let sql = format!(
        "
        INSERT INTO {table} (id, encrypted_text)
        VALUES ($1, $2), ($3, $4), ($5, $6)
    "
    );

    client
        .query(
            &sql,
            &[
                &random_id(),
                &s_two,
                &random_id(),
                &s_one,
                &random_id(),
                &s_three,
            ],
        )
        .await
        .unwrap();

    let sql = format!("SELECT encrypted_text FROM {table} e ORDER BY e.encrypted_text");
    let rows = client.query(&sql, &[]).await.unwrap();

    let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<String>>();
    let expected = vec![s_one, s_two, s_three];

    assert_eq!(actual, expected);
}

/// ORDER BY column not in SELECT projection.
pub async fn ore_order_no_eql_column_in_select_projection(
    client: &tokio_postgres::Client,
    table: &str,
) {
    let id_one = random_id();
    let s_one = "a";
    let id_two = random_id();
    let s_two = "b";
    let id_three = random_id();
    let s_three = "c";

    let sql = format!(
        "
        INSERT INTO {table} (id, encrypted_text)
        VALUES ($1, $2), ($3, $4), ($5, $6)
    "
    );

    client
        .query(
            &sql,
            &[&id_two, &s_two, &id_one, &s_one, &id_three, &s_three],
        )
        .await
        .unwrap();

    let sql = format!("SELECT id FROM {table} ORDER BY encrypted_text");
    let rows = client.query(&sql, &[]).await.unwrap();

    let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<i64>>();
    let expected = vec![id_one, id_two, id_three];

    assert_eq!(actual, expected);
}

/// Plaintext column ordering (sanity check).
pub async fn ore_order_plaintext_column(client: &tokio_postgres::Client, table: &str) {
    let s_one = "a";
    let s_two = "b";
    let s_three = "c";

    let sql = format!(
        "
        INSERT INTO {table} (id, plaintext)
        VALUES ($1, $2), ($3, $4), ($5, $6)
    "
    );

    client
        .query(
            &sql,
            &[
                &random_id(),
                &s_two,
                &random_id(),
                &s_one,
                &random_id(),
                &s_three,
            ],
        )
        .await
        .unwrap();

    let sql = format!("SELECT plaintext FROM {table} ORDER BY plaintext");
    let rows = client.query(&sql, &[]).await.unwrap();

    let actual = rows.iter().map(|row| row.get(0)).collect::<Vec<String>>();
    let expected = vec![s_one, s_two, s_three];

    assert_eq!(actual, expected);
}

/// Mixed plaintext + encrypted column ordering.
pub async fn ore_order_plaintext_and_eql_columns(client: &tokio_postgres::Client, table: &str) {
    let s_plaintext_one = "a";
    let s_plaintext_two = "a";
    let s_plaintext_three = "b";

    let s_encrypted_one = "a";
    let s_encrypted_two = "b";
    let s_encrypted_three = "c";

    let sql = format!(
        "
        INSERT INTO {table} (id, plaintext, encrypted_text)
        VALUES ($1, $2, $3), ($4, $5, $6), ($7, $8, $9)
    "
    );

    client
        .query(
            &sql,
            &[
                &random_id(),
                &s_plaintext_two,
                &s_encrypted_two,
                &random_id(),
                &s_plaintext_one,
                &s_encrypted_one,
                &random_id(),
                &s_plaintext_three,
                &s_encrypted_three,
            ],
        )
        .await
        .unwrap();

    let sql = format!(
        "SELECT plaintext, encrypted_text FROM {table} ORDER BY plaintext, encrypted_text"
    );
    let rows = client.query(&sql, &[]).await.unwrap();

    let actual = rows
        .iter()
        .map(|row| (row.get(0), row.get(1)))
        .collect::<Vec<(&str, &str)>>();

    let expected = vec![
        (s_plaintext_one, s_encrypted_one),
        (s_plaintext_two, s_encrypted_two),
        (s_plaintext_three, s_encrypted_three),
    ];

    assert_eq!(actual, expected);
}

/// Simple query protocol ordering.
pub async fn ore_order_simple_protocol(client: &tokio_postgres::Client, table: &str) {
    let sql = format!(
        "INSERT INTO {table} (id, encrypted_text) VALUES ({}, 'y'), ({}, 'x'), ({}, 'z')",
        random_id(),
        random_id(),
        random_id()
    );

    client.simple_query(&sql).await.unwrap();

    let sql = format!("SELECT encrypted_text FROM {table} ORDER BY encrypted_text");
    let rows = client.simple_query(&sql).await.unwrap();

    let actual = rows
        .iter()
        .filter_map(|row| {
            if let SimpleQueryMessage::Row(row) = row {
                row.get(0)
            } else {
                None
            }
        })
        .collect::<Vec<_>>();

    let expected = vec!["x", "y", "z"];

    assert_eq!(actual, expected);
}

/// Generic ORE ordering test for numeric types.
///
/// `values` must be provided in ascending sorted order.
/// Values are inserted in interleaved (non-sorted) order, then verified
/// via ORDER BY in the given direction.
pub async fn ore_order_generic<T>(
    client: &tokio_postgres::Client,
    table: &str,
    col_name: &str,
    values: Vec<T>,
    direction: SortDirection,
) where
    for<'a> T: Clone + PartialEq + ToSql + Sync + FromSql<'a> + PartialOrd + Debug,
{
    let insert_sql = format!("INSERT INTO {table} (id, {col_name}) VALUES ($1, $2)");

    for idx in interleaved_indices(values.len()) {
        client
            .query(&insert_sql, &[&random_id(), &values[idx]])
            .await
            .unwrap();
    }

    let dir = direction.as_sql();
    let select_sql = format!("SELECT {col_name} FROM {table} ORDER BY {col_name} {dir}");
    let rows = client.query(&select_sql, &[]).await.unwrap();

    let actual: Vec<T> = rows.iter().map(|row| row.get(0)).collect();

    let expected: Vec<T> = if matches!(direction, SortDirection::Desc) {
        values.into_iter().rev().collect()
    } else {
        values
    };

    assert_eq!(actual, expected);
}
