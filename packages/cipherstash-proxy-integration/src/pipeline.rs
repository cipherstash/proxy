#[cfg(test)]
mod tests {
    use std::sync::{Arc, RwLock};

    use crate::common::{clear, connect_with_tls, id, trace, PROXY};

    #[tokio::test]
    async fn pipeline() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let text_id = id();
        let encrypted_text = "hello@cipherstash.com";

        let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
        client
            .query(sql, &[&text_id, &encrypted_text])
            .await
            .expect("ok");

        let int2_id = id();
        let encrypted_int2: i16 = 16;

        let sql = "INSERT INTO encrypted (id, encrypted_int2) VALUES ($1, $2)";
        client
            .query(sql, &[&int2_id, &encrypted_int2])
            .await
            .expect("ok");

        let int4_id = id();
        let encrypted_int4: i32 = 32;

        let sql = "INSERT INTO encrypted (id, encrypted_int4) VALUES ($1, $2)";
        client
            .query(sql, &[&int4_id, &encrypted_int4])
            .await
            .expect("ok");

        let plaintext_id = id();
        let plaintext_text = "blahvtha";

        let sql = "INSERT INTO encrypted (id, plaintext) VALUES ($1, $2)";
        client
            .query(sql, &[&plaintext_id, &plaintext_text])
            .await
            .expect("ok");

        let counter = Arc::new(RwLock::new(0));

        let text = async {
            let sql = "SELECT id, encrypted_text FROM encrypted WHERE id = $1";
            let rows = client.query(sql, &[&text_id]).await.expect("ok");

            assert!(rows.len() == 1);

            for row in rows {
                let result: String = row.get("encrypted_text");
                assert_eq!(encrypted_text, result);

                let _ = counter.write().map(|mut c| *c += 1);
            }
        };

        let int2 = async {
            let sql = "SELECT id, encrypted_int2 FROM encrypted WHERE id = $1";
            let rows = client.query(sql, &[&int2_id]).await.expect("ok");

            assert!(rows.len() == 1);

            for row in rows {
                let result: i16 = row.get("encrypted_int2");
                assert_eq!(encrypted_int2, result);

                let _ = counter.write().map(|mut c| *c += 1);
            }
        };

        let int4 = async {
            let sql = "SELECT id, encrypted_int4 FROM encrypted WHERE id = $1";
            let rows = client.query(sql, &[&int4_id]).await.expect("ok");

            assert!(rows.len() == 1);

            for row in rows {
                let result: i32 = row.get("encrypted_int4");
                assert_eq!(encrypted_int4, result);

                let _ = counter.write().map(|mut c| *c += 1);
            }
        };

        let plaintext = async {
            let sql = "SELECT id, plaintext FROM encrypted WHERE id = $1";
            let rows = client.query(sql, &[&plaintext_id]).await.expect("ok");

            assert!(rows.len() == 1);

            for row in rows {
                let result: String = row.get("plaintext");
                assert_eq!(plaintext_text, result);

                let _ = counter.write().map(|mut c| *c += 1);
            }
        };

        tokio::join!(text, plaintext, int2, int4);

        let count = counter.read().ok().map(|c| *c).unwrap();
        assert_eq!(count, 4);
    }
}
