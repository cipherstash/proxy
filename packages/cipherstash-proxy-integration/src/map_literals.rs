#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, id, trace, PROXY};

    #[tokio::test]
    async fn map_literal() {
        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_text = "hello@cipherstash.com";

        let sql =
            format!("INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, '{encrypted_text}')");
        client.query(&sql, &[]).await.expect("INSERT query failed");

        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {id}");
        let rows = client
            .query(&sql, &[])
            .await
            .expect("SELECT query failed");

        let result: String = rows[0].get("encrypted_text");
        assert_eq!(encrypted_text, result);
    }

    #[tokio::test]
    async fn map_literal_with_param() {
        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_text = "hello@cipherstash.com";
        let int2: i16 = 1;

        let sql =
            format!("INSERT INTO encrypted (id, encrypted_text, encrypted_bool, encrypted_int2) VALUES ({id}, '{encrypted_text}', $1, $2)");
        client.query(&sql, &[&true, &int2]).await.expect("INSERT query failed");

        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {id}");
        let rows = client
            .query(&sql, &[])
            .await
            .expect("SELECT query failed");

        println!("encrypted: {:?}", rows[0])
    }
}
