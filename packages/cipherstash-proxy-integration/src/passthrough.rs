#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, id, random_string, PROXY};
    use rand::Rng;
    use std::error::Error;

    #[tokio::test]
    async fn passthrough_statement() {
        let client = connect_with_tls(PROXY).await;

        let id = id();
        let encrypted_text = "hello@cipherstash.com";

        let sql = "INSERT INTO plaintext (id, plaintext) VALUES ($1, $2)";
        client
            .query(sql, &[&id, &encrypted_text])
            .await
            .expect("ok");

        let sql = "SELECT id, plaintext FROM plaintext WHERE id = $1";
        let rows = client.query(sql, &[&id]).await.expect("ok");

        assert!(rows.len() == 1);

        for row in rows {
            let result: String = row.get("plaintext");
            assert_eq!(encrypted_text, result);
        }
    }

    #[tokio::test]
    async fn passthrough_invalid_statement() {
        let client = connect_with_tls(PROXY).await;

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
                    let id = id();
                    let encrypted_text = random_string();

                    let sql = "INSERT INTO plaintext (id,  plaintext) VALUES ($1, $2)";
                    client
                        .query(sql, &[&id, &encrypted_text])
                        .await
                        .expect("ok");

                    let sql = "SELECT id, plaintext FROM plaintext WHERE id = $1";
                    let rows = client.query(sql, &[&id]).await.expect("ok");

                    assert!(rows.len() == 1);

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
}
