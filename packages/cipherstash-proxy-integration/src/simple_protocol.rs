#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, database_config_with_port, id, PROXY};
    use tokio_postgres::SimpleQueryMessage::{CommandComplete, Row};

    #[tokio::test]
    async fn simple_text_without_encryption() {
        let config = database_config_with_port(PROXY);
        let client = connect_with_tls(&config).await;
        let id = id();
        let sql = format!("INSERT INTO encrypted (id, plaintext) VALUES ({id}, 'plain')");
        client
            .simple_query(&sql)
            .await
            .expect("INSERT query failed");

        let sql = format!("SELECT id, plaintext FROM encrypted WHERE id = {id}");
        let rows = client
            .simple_query(&sql)
            .await
            .expect("SELECT query failed");
        if let Row(r) = &rows[1] {
            assert_eq!(Some("plain"), r.get(1));
        } else {
            panic!("Unexpected query results: {:?}", rows);
        }
    }

    #[tokio::test]
    async fn simple_text_with_encryption() {
        let config = database_config_with_port(PROXY);
        let client = connect_with_tls(&config).await;

        let id = id();
        let encrypted_text = "hello@cipherstash.com";

        let sql =
            format!("INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, '{encrypted_text}')");
        let insert_result = client
            .simple_query(&sql)
            .await
            .expect("INSERT query failed");

        // CmmandComplete does not implement PartialEq, so no equality check with ==
        match &insert_result[0] {
            CommandComplete(n) => assert_eq!(1, *n),
            _unexpected => panic!("unexpected insert result: {:?}", insert_result),
        }

        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {id}");
        let rows = client
            .simple_query(&sql)
            .await
            .expect("SELECT query failed");

        if let Row(r) = &rows[1] {
            assert_eq!(Some(id.to_string().as_str()), r.get(0));
            assert_eq!(Some("'hello@cipherstash.com'"), r.get(1));
        } else {
            panic!("Row(row) expected but got: {:?}", &rows[1]);
        }

        if let CommandComplete(n) = &rows[2] {
            assert_eq!(1, *n);
        } else {
            panic!("CommandComplete(1) expected but got: {:?}", &rows[2]);
        }
    }
}
