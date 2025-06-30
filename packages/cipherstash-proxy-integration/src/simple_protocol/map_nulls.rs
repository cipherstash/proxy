#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, random_id, trace, PROXY};
    use tokio_postgres::SimpleQueryMessage::Row;

    #[tokio::test]
    async fn map_simple_null_literal() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = random_id();
        let encrypted_text: Option<&str> = None;

        let sql = format!("INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, NULL)");
        client.simple_query(&sql).await.unwrap();

        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {id}");
        let rows = client.simple_query(&sql).await.unwrap();

        assert!(!rows.is_empty());

        if let Row(r) = &rows[1] {
            assert_eq!(encrypted_text, r.get(1));
        } else {
            panic!("Unexpected query results: {rows:?}");
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
            panic!("Unexpected query results: {rows:?}");
        }
    }

    #[tokio::test]
    async fn map_simple_many_null_literals() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let id = random_id();

        let sql = format!("INSERT INTO encrypted (id, encrypted_text, encrypted_int2, encrypted_int4, encrypted_int8) VALUES ({id}, NULL, NULL, NULL, NULL)");
        client.simple_query(&sql).await.unwrap();

        let sql = format!("SELECT id, encrypted_text, encrypted_int2, encrypted_int4, encrypted_int8 FROM encrypted WHERE id = {id}");
        let rows = client.simple_query(&sql).await.unwrap();

        assert!(!rows.is_empty());

        if let Row(r) = &rows[1] {
            assert!(r.get(1).is_none());
            assert!(r.get(2).is_none());
            assert!(r.get(3).is_none());
            assert!(r.get(4).is_none());
        } else {
            panic!("Unexpected query results: {rows:?}");
        }

        let sql = format!("UPDATE encrypted SET encrypted_float8 = NULL WHERE id = {id}");
        client.simple_query(&sql).await.unwrap();

        let sql = format!("SELECT id, encrypted_text, encrypted_int2, encrypted_int4, encrypted_int8, encrypted_float8 FROM encrypted WHERE id = {id}");
        let rows = client.simple_query(&sql).await.unwrap();

        assert!(!rows.is_empty());

        if let Row(r) = &rows[1] {
            assert!(r.get(1).is_none());
            assert!(r.get(2).is_none());
            assert!(r.get(3).is_none());
            assert!(r.get(4).is_none());
            assert!(r.get(5).is_none());
        } else {
            panic!("Unexpected query results: {rows:?}");
        }
    }
}
