//! JSON selector params when the client declares Parse types.
//!
//! A JSON field selector is passed to the rewritten `eql_v3` function as bare
//! encrypted **text** — `eql_v3."->"(json, text)`, `eql_v3.jsonb_path_exists(json,
//! text)` and friends. If Proxy declares the selector param as `jsonb` (the wire
//! type of every other encrypted operand), PostgreSQL cannot find the function
//! and rejects the rewritten Parse.
//!
//! This only shows up when the client sends its own param OIDs in Parse, which
//! is what pgx does in `cache_describe` mode — hence the `prepare_typed` here.
//! With no declared types PostgreSQL infers them and the bug is invisible.

#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, insert_jsonb, trace, PROXY};
    use crate::support::json_path::JsonPath;
    use tokio_postgres::types::Type;

    #[tokio::test]
    async fn jsonb_path_exists_with_declared_selector_type() {
        trace();
        clear().await;
        insert_jsonb().await;

        let client = connect_with_tls(*PROXY).await;
        let selector = JsonPath::new("$.number");

        let sql = "SELECT jsonb_path_exists(encrypted_jsonb, $1) FROM encrypted";
        let stmt = client
            .prepare_typed(sql, &[Type::TEXT])
            .await
            .expect("declared-type prepare of a JSON selector param should succeed");

        let rows = client.query(&stmt, &[&selector]).await.unwrap();
        let actual: Vec<bool> = rows.iter().map(|r| r.get(0)).collect();
        assert_eq!(vec![true], actual);
    }

    #[tokio::test]
    async fn jsonb_path_query_first_with_declared_selector_type() {
        trace();
        clear().await;
        insert_jsonb().await;

        let client = connect_with_tls(*PROXY).await;
        let selector = JsonPath::new("$.string");

        let sql = "SELECT jsonb_path_query_first(encrypted_jsonb, $1) FROM encrypted";
        let stmt = client
            .prepare_typed(sql, &[Type::TEXT])
            .await
            .expect("declared-type prepare of a JSON selector param should succeed");

        let rows = client.query(&stmt, &[&selector]).await.unwrap();
        assert_eq!(1, rows.len());
    }

    #[tokio::test]
    async fn jsonb_field_access_with_declared_selector_type() {
        trace();
        clear().await;
        insert_jsonb().await;

        let client = connect_with_tls(*PROXY).await;

        let sql = "SELECT encrypted_jsonb -> $1 FROM encrypted";
        let stmt = client
            .prepare_typed(sql, &[Type::TEXT])
            .await
            .expect("declared-type prepare of a JSON selector param should succeed");

        let rows = client.query(&stmt, &[&"string"]).await.unwrap();
        assert_eq!(1, rows.len());
    }
}
