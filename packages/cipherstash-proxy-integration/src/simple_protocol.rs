#[cfg(test)]
mod tests {
    use std::process::Command;

    use crate::common::{connect_with_tls, database_config_with_port, id, PROXY};
    use tokio::runtime;
    use tokio_postgres::SimpleQueryMessage::{CommandComplete, Row, RowDescription};

    #[tokio::test]
    async fn simple_text() {
        let config = database_config_with_port(PROXY);
        let client = connect_with_tls(&config).await;

        let id = id();
        let encrypted_text = "hello@cipherstash.com";

        let sql =
            format!("INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, '{encrypted_text}')");
        let insert_result = client.simple_query(&sql).await.expect("ok");

        // CmmandComplete does not implement PartialEq, so no equality check with ==
        match &insert_result[0] {
            CommandComplete(n) => assert_eq!(1, *n),
            _unexpected => panic!("unexpected insert result: {:?}", insert_result),
        }

        let sql = format!("SELECT id, encrypted_text FROM encrypted WHERE id = {id}");
        let rows = client.simple_query(&sql).await.expect("ok");

        let mut complete_message_count = 0;
        let mut row_description_count = 0;
        let mut row_count = 0;
        for row in rows {
            match row {
                // 1 row should be affected
                CommandComplete(n) => {
                    assert_eq!(n, 1);
                    complete_message_count += 1;
                }
                RowDescription(_) => {
                    row_description_count += 1;
                }
                Row(r) => {
                    row_count += 1;
                    let fetched_id = r.get(0);
                    let fetched_text = r.get(1);
                    assert_eq!(Some(id.to_string().as_str()), fetched_id);
                    assert_eq!(Some("'hello@cipherstash.com'"), fetched_text);
                }
                unexpected => panic!("unexpected in rows: {:?}", unexpected),
            }
        }
        assert_eq!(1, complete_message_count);
        assert_eq!(1, row_description_count);
        assert_eq!(1, row_count);
    }
}
