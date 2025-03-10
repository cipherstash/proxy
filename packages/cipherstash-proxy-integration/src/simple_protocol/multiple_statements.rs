#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, id, trace, PROXY};
    use fake::{Fake, Faker};
    use tokio_postgres::SimpleQueryMessage::CommandComplete;

    #[tokio::test]
    async fn multiple_inserts_with_text() {
        trace();
        clear().await;

        let client = connect_with_tls(PROXY).await;

        let data = (0..5)
            .map(|_| (id(), Faker.fake::<String>()))
            .collect::<Vec<_>>();

        // Build SQL string containing multiple statements;
        let sql: String = data
            .iter()
            .map(|(id, encrypted_text)| {
                format!(
                    "INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, '{encrypted_text}')"
                )
            })
            .collect::<Vec<_>>()
            .join(";");

        let insert_result = client.simple_query(&sql).await.unwrap();

        // CmmandComplete does not implement PartialEq, so no equality check with ==
        match &insert_result[0] {
            CommandComplete(n) => assert_eq!(1, *n),
            _unexpected => panic!("unexpected insert result: {:?}", insert_result),
        }

        // Check each Row by ID
        for (id, encrypted_text) in data {
            let sql = "SELECT id, encrypted_text FROM encrypted WHERE id = $1";
            let rows = client.query(sql, &[&id]).await.unwrap();

            assert_eq!(rows.len(), 1);

            for row in rows {
                let result: String = row.get("encrypted_text");
                assert_eq!(encrypted_text, result);
            }
        }
    }

    #[tokio::test]
    async fn multiple_inserts_and_updates_with_text() {
        trace();
        clear().await;

        let client = connect_with_tls(PROXY).await;

        let data = (0..5)
            .map(|_| (id(), Faker.fake::<String>(), Faker.fake::<String>()))
            .collect::<Vec<_>>();

        // Build SQL string containing multiple statements;
        let sql: String = data
            .iter()
            .map(|(id, encrypted_text, _)| {
                format!(
                    "INSERT INTO encrypted (id, encrypted_text) VALUES ({id}, '{encrypted_text}')"
                )
            })
            .collect::<Vec<_>>()
            .join(";");

        let insert_result = client.simple_query(&sql).await.unwrap();

        // CmmandComplete does not implement PartialEq, so no equality check with ==
        match &insert_result[0] {
            CommandComplete(n) => assert_eq!(1, *n),
            _unexpected => panic!("unexpected insert result: {:?}", insert_result),
        }

        // Build SQL string containing multiple statements;
        let sql: String = data
            .iter()
            .map(|(id, _, encrypted_text)| {
                format!("UPDATE encrypted SET encrypted_text = '{encrypted_text}' WHERE id = {id}")
            })
            .collect::<Vec<_>>()
            .join(";");

        let insert_result = client.simple_query(&sql).await.unwrap();

        // CmmandComplete does not implement PartialEq, so no equality check with ==
        match &insert_result[0] {
            CommandComplete(n) => assert_eq!(1, *n),
            _unexpected => panic!("unexpected insert result: {:?}", insert_result),
        }

        // Check each Row by ID
        for (id, _, encrypted_text) in data {
            let sql = "SELECT id, encrypted_text FROM encrypted WHERE id = $1";
            let rows = client.query(sql, &[&id]).await.unwrap();

            assert_eq!(rows.len(), 1);

            for row in rows {
                let result: String = row.get("encrypted_text");
                assert_eq!(encrypted_text, result);
            }
        }
    }
}
