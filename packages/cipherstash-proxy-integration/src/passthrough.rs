#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, random_id, random_string, PROXY};
    use rand::Rng;
    use std::error::Error;

    #[tokio::test]
    async fn passthrough_statement() {
        let client = connect_with_tls(PROXY).await;

        clear().await;

        let id = random_id();
        let encrypted_text = "hello@cipherstash.com";

        let sql = "INSERT INTO plaintext (id, plaintext) VALUES ($1, $2)";
        client.query(sql, &[&id, &encrypted_text]).await.unwrap();

        let sql = "SELECT id, plaintext FROM plaintext WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let result: String = row.get("plaintext");
            assert_eq!(encrypted_text, result);
        }
    }

    #[tokio::test]
    async fn passthrough_invalid_statement() {
        let client = connect_with_tls(PROXY).await;

        clear().await;

        let sql = "SELECT * FROM blahvtha";
        let result = client.query(sql, &[]).await;

        assert!(result.is_err());

        match result {
            Ok(_) => unreachable!(),
            Err(error) => match error.source() {
                Some(db_error) => {
                    assert_eq!(
                        db_error.to_string(),
                        "ERROR: relation \"blahvtha\" does not exist"
                    );
                }
                None => unreachable!(),
            },
        }
    }

    #[tokio::test]
    async fn passthrough_statement_parallel() {
        for _x in 1..100 {
            tokio::spawn(async move {
                let client = connect_with_tls(PROXY).await;

                for _x in 1..10 {
                    let id = random_id();
                    let encrypted_text = random_string();

                    let sql = "INSERT INTO plaintext (id,  plaintext) VALUES ($1, $2)";
                    client.query(sql, &[&id, &encrypted_text]).await.unwrap();

                    let sql = "SELECT id, plaintext FROM plaintext WHERE id = $1";
                    let rows = client.query(sql, &[&id]).await.unwrap();

                    assert_eq!(rows.len(), 1);

                    for row in rows {
                        let result: String = row.get("plaintext");
                        assert_eq!(encrypted_text, result);
                    }
                }
            })
            .await
            .unwrap();

            let sleep_duration = rand::rng().random_range(1..=10);
            tokio::time::sleep(std::time::Duration::from_millis(sleep_duration)).await;
        }
    }

    #[tokio::test]
    async fn passthrough_insert_from_select() {
        let client = connect_with_tls(PROXY).await;

        clear().await;

        // Setup data
        let id_1 = random_id();
        let plaintext = "hello@cipherstash.com";

        let sql = "INSERT INTO plaintext (id, plaintext) VALUES ($1, $2)";
        client.query(sql, &[&id_1, &plaintext]).await.unwrap();

        let id_2 = id_1 + 1;

        // Insert value is selected from the record we just created
        let select =
            "SELECT id + 1, plaintext FROM plaintext WHERE id = ANY( ARRAY[ $1::Int8, $2::Int8 ] )";
        let sql = format!("INSERT INTO plaintext (id, plaintext) {select} ON CONFLICT DO NOTHING");
        client.query(&sql, &[&id_1, &id_2]).await.unwrap();

        let sql = "SELECT id, plaintext FROM plaintext WHERE id = $1";
        let rows = client.query(sql, &[&id_2]).await.unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let result: String = row.get("plaintext");
            assert_eq!(plaintext, result);
        }
    }

    #[tokio::test]
    async fn passthrough_insert_with_value_from_select() {
        let client = connect_with_tls(PROXY).await;

        clear().await;

        // Setup data
        let id_1 = random_id();
        let plaintext = "hello@cipherstash.com";

        let sql = "INSERT INTO plaintext (id, plaintext) VALUES ($1, $2)";
        client.query(sql, &[&id_1, &plaintext]).await.unwrap();

        let id_2 = random_id();

        // Insert value is selected from the record we just created
        let select = "SELECT plaintext FROM plaintext WHERE id = $2";
        let sql = format!("INSERT INTO plaintext (id, plaintext) VALUES ($1, ({select}))");
        client.query(&sql, &[&id_2, &id_1]).await.unwrap();

        let sql = "SELECT id, plaintext FROM plaintext WHERE id = $1";
        let rows = client.query(sql, &[&id_2]).await.unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let result: String = row.get("plaintext");
            assert_eq!(plaintext, result);
        }
    }

    #[tokio::test]
    async fn passthrough_insert_with_returning() {
        let client = connect_with_tls(PROXY).await;

        clear().await;

        // Setup data
        let id = random_id();
        let plaintext = "hello@cipherstash.com";

        let sql = "INSERT INTO plaintext (id, plaintext) VALUES ($1, $2) RETURNING *";
        let rows = client.query(sql, &[&id, &plaintext]).await.unwrap();

        assert_eq!(rows.len(), 1);

        for row in rows {
            let result: String = row.get("plaintext");
            assert_eq!(plaintext, result);
        }
    }

    #[tokio::test]
    async fn passthrough_select_with_cardinality() {
        let client = connect_with_tls(PROXY).await;

        clear().await;

        // Setup data
        let id_1 = random_id();
        let plaintext = "hello@cipherstash.com";

        let sql = "INSERT INTO plaintext (id, plaintext) VALUES ($1, $2)";
        client.query(sql, &[&id_1, &plaintext]).await.unwrap();

        let id_2 = random_id();

        let sql = "INSERT INTO plaintext (id) VALUES ($1)";
        client.query(sql, &[&id_2]).await.unwrap();

        let sql = "SELECT ARRAY_REMOVE(ARRAY_AGG(id), NULL), plaintext
                         FROM plaintext
                         WHERE CARDINALITY(ARRAY[1,2]) <> 0
                         GROUP BY plaintext";

        let rows = client.query(sql, &[]).await.unwrap();

        assert_eq!(rows.len(), 2);
    }

    #[tokio::test]
    async fn passthrough_delete_with_select() {
        let client = connect_with_tls(PROXY).await;

        clear().await;

        // Setup data
        let id_1 = random_id();
        let plaintext = "one@cipherstash.com";

        let sql = "INSERT INTO plaintext (id, plaintext) VALUES ($1, $2)";
        client.query(sql, &[&id_1, &plaintext]).await.unwrap();

        let id_2 = random_id();
        let plaintext = "two@cipherstash.com";

        let sql = "INSERT INTO plaintext (id, plaintext) VALUES ($1, $2)";
        client.query(sql, &[&id_2, &plaintext]).await.unwrap();

        let sql = "DELETE FROM plaintext
                         WHERE id IN (SELECT id FROM plaintext WHERE plaintext = $1)";
        client.query(sql, &[&plaintext]).await.unwrap();

        let sql = "SELECT * FROM plaintext WHERE plaintext = $1";
        let rows = client.query(sql, &[&plaintext]).await.unwrap();

        assert_eq!(rows.len(), 0);
    }
}
