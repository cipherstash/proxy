#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, id, trace, PROXY};
    use tokio_postgres::SimpleQueryMessage::{CommandComplete, Row};
    use tracing::info;

    #[tokio::test]
    async fn map_insert_null_param() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_text: Option<String> = None;

        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        client.query(sql, &[&id, &encrypted_text]).await.unwrap();

        let sql = "SELECT id, encrypted_text FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let result: Option<String> = row.get("encrypted_text");
            assert_eq!(encrypted_text, result);
        }

        let encrypted_int4: Option<i32> = None;
        let sql = "UPDATE encrypted SET encrypted_int4 = $1 WHERE id = $2";
        client.query(sql, &[&encrypted_int4, &id]).await.unwrap();

        let sql = "SELECT id, encrypted_int4 FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let result: Option<i32> = row.get("encrypted_int4");
            assert_eq!(encrypted_int4, result);
        }
    }

    #[tokio::test]
    async fn map_update_null_param() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_text = "hello@cipherstash.com";

        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        client.query(sql, &[&id, &encrypted_text]).await.unwrap();

        let sql = "SELECT id, encrypted_text FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let result: String = row.get("encrypted_text");
            assert_eq!(encrypted_text, result);
        }

        // Update the encrypted_text to NULL
        let encrypted_text: Option<String> = None;
        let sql = "UPDATE encrypted SET encrypted_text = $1 WHERE id = $2";
        client.query(sql, &[&encrypted_text, &id]).await.unwrap();

        let sql = "SELECT id, encrypted_text FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let result: Option<String> = row.get("encrypted_text");
            assert_eq!(encrypted_text, result);
        }
    }

    #[tokio::test]
    async fn map_insert_null_literal() {
        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_text = "hello@cipherstash.com";

        let sql = format!("INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, NULL)");

        client.simple_query(&sql).await.unwrap();

        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {id}");
        let rows = client.simple_query(&sql).await.unwrap();

        if let Row(r) = &rows[1] {
            assert_eq!(Some("plain"), r.get(1));
        } else {
            panic!("Unexpected query results: {:?}", rows);
        }
    }
}
