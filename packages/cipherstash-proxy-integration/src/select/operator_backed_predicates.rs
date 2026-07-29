//! Predicates that reduce to EQL's own operator overloads.
//!
//! EQL v3 ships `=`, `<`, `<=`, `>`, `>=` for every encrypted domain, each
//! implemented as a comparison of the relevant term — `eql_v3.eq` is
//! `eq_term(a) = eq_term(b)`. Any shape PostgreSQL desugars to those operators
//! is therefore correct without the mapper rewriting it, even though the SQL
//! Proxy emits looks like a raw comparison against the payload.
//!
//! `IN`/`NOT IN` is the same story and is covered in [`super::select_where_in`].
//!
//! These are regression guards: nothing here is currently rewritten, so the
//! tests fail if either the EQL operator overloads or the literal encryption is
//! lost.
//!
//! Contrast [`super::operator_class_shapes`], where the shape reaches the
//! type's default btree/hash operator class instead — operator overloads do not
//! apply there, and those shapes are genuinely broken.

#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, execute_query, random_id, trace, PROXY};

    async fn insert_rows(rows: &[(&str, i32)]) {
        for (text, int4) in rows {
            let id = random_id();
            execute_query(
                "INSERT INTO encrypted (id, encrypted_text, encrypted_int4) VALUES ($1, $2, $3)",
                &[&id, &text.to_string(), int4],
            )
            .await;
        }
    }

    /// `BETWEEN` desugars to `>= AND <=`, both of which EQL overloads.
    #[tokio::test]
    async fn select_where_between_returns_the_range() {
        trace();
        clear().await;

        insert_rows(&[("a", 1), ("b", 2), ("c", 3), ("d", 4), ("e", 5)]).await;

        let client = connect_with_tls(PROXY).await;

        let sql = "SELECT encrypted_int4 FROM encrypted WHERE encrypted_int4 BETWEEN 2 AND 4 \
                   ORDER BY encrypted_int4";
        let rows = client.query(sql, &[]).await.unwrap();
        let actual: Vec<i32> = rows.iter().map(|r| r.get("encrypted_int4")).collect();
        assert_eq!(vec![2, 3, 4], actual);

        // And the complement, so a predicate that matched everything would fail.
        let sql = "SELECT encrypted_int4 FROM encrypted WHERE encrypted_int4 NOT BETWEEN 2 AND 4 \
                   ORDER BY encrypted_int4";
        let rows = client.query(sql, &[]).await.unwrap();
        let actual: Vec<i32> = rows.iter().map(|r| r.get("encrypted_int4")).collect();
        assert_eq!(vec![1, 5], actual);
    }

    /// `IS DISTINCT FROM` is equality with NULL-safe semantics, so it resolves
    /// to the same overload as `=`. Equal plaintexts must compare as *not*
    /// distinct, despite their ciphertexts differing.
    #[tokio::test]
    async fn select_where_is_distinct_from_compares_plaintexts() {
        trace();
        clear().await;

        insert_rows(&[("apple", 1), ("banana", 2), ("cherry", 3)]).await;

        let client = connect_with_tls(PROXY).await;

        let sql =
            "SELECT encrypted_text FROM encrypted WHERE encrypted_text IS DISTINCT FROM 'apple'";
        let rows = client.query(sql, &[]).await.unwrap();
        let mut actual: Vec<String> = rows.iter().map(|r| r.get("encrypted_text")).collect();
        actual.sort();
        assert_eq!(vec!["banana", "cherry"], actual);

        let sql =
            "SELECT encrypted_text FROM encrypted WHERE encrypted_text IS NOT DISTINCT FROM 'apple'";
        let rows = client.query(sql, &[]).await.unwrap();
        let actual: Vec<String> = rows.iter().map(|r| r.get("encrypted_text")).collect();
        assert_eq!(vec!["apple"], actual);
    }
}
