#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, id, trace, PROXY};
    use tokio_postgres::SimpleQueryMessage::Row;

    #[tokio::test]
    async fn map_simple_insert_null_literal() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_text: Option<&str> = None;

        let sql = format!("INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, NULL)");
        client.simple_query(&sql).await.unwrap();

        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {id}");
        let rows = client.simple_query(&sql).await.unwrap();

        assert!(!rows.is_empty());

        if let Row(r) = &rows[1] {
            assert_eq!(encrypted_text, r.get(1));
        } else {
            panic!("Unexpected query results: {:?}", rows);
        }

        let encrypted_int4: Option<&str> = None;

        let sql = format!("UPDATE encrypted SET encrypted_int4 = NULL WHERE id = {id}");
        client.simple_query(&sql).await.unwrap();

        let sql = format!("SELECT id, encrypted_int4 FROM encrypted WHERE id = {id}");
        let rows = client.simple_query(&sql).await.unwrap();

        assert!(!rows.is_empty());

        if let Row(r) = &rows[1] {
            assert_eq!(encrypted_int4, r.get(1));
        } else {
            panic!("Unexpected query results: {:?}", rows);
        }
    }
}
