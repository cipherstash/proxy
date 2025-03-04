#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, id, trace, PROXY};

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
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_text: Option<String> = None;

        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, NULL)";
        client.query(sql, &[&id]).await.unwrap();

        let sql = "SELECT id, encrypted_text FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let result_id: i64 = row.get("id");
            assert_eq!(id, result_id);

            let result: Option<String> = row.get("encrypted_text");
            assert_eq!(encrypted_text, result);
        }
    }

    #[tokio::test]
    async fn map_insert_null_literal_with_param() {
        trace();

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_text: Option<String> = None;
        let encrypted_int2: i16 = 42;

        let sql =
            "INSERT INTO encrypted (id, encrypted_text, encrypted_int2) VALUES ($1, NULL, $2)";
        client.query(sql, &[&id, &encrypted_int2]).await.unwrap();

        let sql = "SELECT id, encrypted_text, encrypted_int2 FROM encrypted WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let result_id: i64 = row.get("id");
            assert_eq!(id, result_id);

            let result_int: i16 = row.get("encrypted_int2");
            assert_eq!(encrypted_int2, result_int);

            let result: Option<String> = row.get("encrypted_text");
            assert_eq!(encrypted_text, result);
        }
    }
}
