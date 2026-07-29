//! `IN` / `NOT IN` on an encrypted column.
//!
//! Unlike `ORDER BY`, `GROUP BY` and `DISTINCT`, `IN` needs no rewrite: it
//! desugars to `= ANY(…)`, and EQL v3 ships `=` overloads for every encrypted
//! domain in both directions. `eql_v3.eq` is
//!
//! ```sql
//! SELECT eql_v3.eq_term(a::public.eql_v3_text_eq) = eql_v3.eq_term(b)
//! ```
//!
//! so the comparison already happens on the equality term. The proxy therefore
//! forwards `enc IN ('<payload>', '<payload>')` unchanged and PostgreSQL
//! resolves it correctly.
//!
//! That is worth pinning down, because the shape *looks* broken from the SQL
//! alone — the payloads carry `c`, the randomised ciphertext, so a raw jsonb
//! comparison would match nothing and `NOT IN` would match everything. What
//! saves it is operator resolution, which is invisible in the rewritten
//! statement. These tests assert the rows, so they hold whether the behaviour
//! comes from EQL's operators (as now) or from an explicit `eq_term` rewrite
//! (if one is ever added), and they fail loudly if either is lost.
//!
//! The distinction is that `ORDER BY`/`GROUP BY`/`DISTINCT` use the type's
//! default btree/hash **operator class** rather than these overloaded
//! operators, which is why those three did need rewriting and this does not.

#[cfg(test)]
mod tests {
    use crate::common::{
        clear, connect_with_tls, execute_query, random_id, simple_query, trace, PROXY,
    };

    /// Inserts one row per value and returns them in insertion order.
    async fn insert_text(values: &[&str]) {
        for value in values {
            let id = random_id();
            execute_query(
                "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
                &[&id, &value.to_string()],
            )
            .await;
        }
    }

    fn sorted(mut values: Vec<String>) -> Vec<String> {
        values.sort();
        values
    }

    /// `IN` returns exactly the rows whose plaintext is in the list.
    #[tokio::test]
    async fn select_where_in_list_of_literals() {
        trace();
        clear().await;

        insert_text(&["apple", "banana", "cherry"]).await;

        let sql =
            "SELECT encrypted_text FROM encrypted WHERE encrypted_text IN ('apple', 'banana')";

        let client = connect_with_tls(*PROXY).await;
        let rows = client.query(sql, &[]).await.unwrap();
        let actual: Vec<String> = rows.iter().map(|r| r.get("encrypted_text")).collect();
        assert_eq!(vec!["apple", "banana"], sorted(actual));

        let actual = simple_query::<String>(sql).await;
        assert_eq!(vec!["apple", "banana"], sorted(actual));
    }

    /// `NOT IN` returns exactly the rows whose plaintext is absent from the list.
    #[tokio::test]
    async fn select_where_not_in_list_of_literals() {
        trace();
        clear().await;

        insert_text(&["apple", "banana", "cherry"]).await;

        let sql =
            "SELECT encrypted_text FROM encrypted WHERE encrypted_text NOT IN ('apple', 'banana')";

        let client = connect_with_tls(*PROXY).await;
        let rows = client.query(sql, &[]).await.unwrap();
        let actual: Vec<String> = rows.iter().map(|r| r.get("encrypted_text")).collect();
        assert_eq!(vec!["cherry"], actual);

        let actual = simple_query::<String>(sql).await;
        assert_eq!(vec!["cherry"], actual);
    }

    /// The same, with the list bound as params rather than written as literals —
    /// the extended protocol path, where each operand is a query operand.
    #[tokio::test]
    async fn select_where_in_list_of_params() {
        trace();
        clear().await;

        insert_text(&["apple", "banana", "cherry"]).await;

        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT encrypted_text FROM encrypted WHERE encrypted_text IN ($1, $2)";
        let rows = client
            .query(sql, &[&"apple".to_string(), &"banana".to_string()])
            .await
            .unwrap();
        let actual: Vec<String> = rows.iter().map(|r| r.get("encrypted_text")).collect();
        assert_eq!(vec!["apple", "banana"], sorted(actual));

        let sql = "SELECT encrypted_text FROM encrypted WHERE encrypted_text NOT IN ($1, $2)";
        let rows = client
            .query(sql, &[&"apple".to_string(), &"banana".to_string()])
            .await
            .unwrap();
        let actual: Vec<String> = rows.iter().map(|r| r.get("encrypted_text")).collect();
        assert_eq!(vec!["cherry"], actual);
    }

    /// A list that matches nothing returns nothing — distinguishing a genuinely
    /// empty result from the "always empty" failure mode, where `IN` returns no
    /// rows whatever the list contains.
    #[tokio::test]
    async fn select_where_in_list_with_no_matches() {
        trace();
        clear().await;

        insert_text(&["apple", "banana"]).await;

        let sql = "SELECT encrypted_text FROM encrypted WHERE encrypted_text IN ('durian')";

        let client = connect_with_tls(*PROXY).await;
        let rows = client.query(sql, &[]).await.unwrap();
        assert!(rows.is_empty());

        // And the complement returns everything, distinguishing it from the
        // "always all rows" failure mode of `NOT IN`.
        let sql = "SELECT encrypted_text FROM encrypted WHERE encrypted_text NOT IN ('durian')";
        let rows = client.query(sql, &[]).await.unwrap();
        let actual: Vec<String> = rows.iter().map(|r| r.get("encrypted_text")).collect();
        assert_eq!(vec!["apple", "banana"], sorted(actual));
    }
}
