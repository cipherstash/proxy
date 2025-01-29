#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, id, trace, PROXY};
    use std::sync::atomic::{AtomicUsize, Ordering};

    ///
    /// Test statement pipelinining
    ///
    /// Executes statements asynchronously.
    ///
    /// Tests encryption, decryption and passthrough workloads
    ///
    #[tokio::test]
    async fn pipelined_statements() {
        trace();

        clear().await;

        let client = connect_with_tls(PROXY).await;

        let counter = AtomicUsize::new(0);

        let text_id = id();
        let encrypted_text = "hello@cipherstash.com";

        let text = async {
            let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
            client
                .query(sql, &[&text_id, &encrypted_text])
                .await
                .expect("ok");

            let _ = counter.fetch_add(1, Ordering::SeqCst);
        };

        let int2_id = id();
        let encrypted_int2: i16 = 16;

        let int2 = async {
            let sql = "INSERT INTO encrypted (id, encrypted_int2) VALUES ($1, $2)";
            client
                .query(sql, &[&int2_id, &encrypted_int2])
                .await
                .expect("ok");

            let _ = counter.fetch_add(1, Ordering::SeqCst);
        };

        let int4_id = id();
        let encrypted_int4: i32 = 32;

        let int4 = async {
            let sql = "INSERT INTO encrypted (id, encrypted_int4) VALUES ($1, $2)";
            client
                .query(sql, &[&int4_id, &encrypted_int4])
                .await
                .expect("ok");

            let _ = counter.fetch_add(1, Ordering::SeqCst);
        };

        let plaintext_id = id();
        let plaintext_text = "blahvtha";

        let plaintext = async {
            let sql = "INSERT INTO encrypted (id, plaintext) VALUES ($1, $2)";
            client
                .query(sql, &[&plaintext_id, &plaintext_text])
                .await
                .expect("ok");

            let _ = counter.fetch_add(1, Ordering::SeqCst);
        };

        tokio::join!(text, plaintext, int2, int4);

        let count = counter.load(Ordering::SeqCst);
        assert_eq!(count, 4);

        let counter = AtomicUsize::new(0);

        let text = async {
            let sql = "SELECT id, encrypted_text FROM encrypted WHERE id = $1";
            let rows = client.query(sql, &[&text_id]).await.expect("ok");

            assert!(rows.len() == 1);

            for row in rows {
                let result: String = row.get("encrypted_text");
                assert_eq!(encrypted_text, result);

                let _ = counter.fetch_add(1, Ordering::SeqCst);
            }
        };

        let int2 = async {
            let sql = "SELECT id, encrypted_int2 FROM encrypted WHERE id = $1";
            let rows = client.query(sql, &[&int2_id]).await.expect("ok");

            assert!(rows.len() == 1);

            for row in rows {
                let result: i16 = row.get("encrypted_int2");
                assert_eq!(encrypted_int2, result);

                let _ = counter.fetch_add(1, Ordering::SeqCst);
            }
        };

        let int4 = async {
            let sql = "SELECT id, encrypted_int4 FROM encrypted WHERE id = $1";
            let rows = client.query(sql, &[&int4_id]).await.expect("ok");

            assert!(rows.len() == 1);

            for row in rows {
                let result: i32 = row.get("encrypted_int4");
                assert_eq!(encrypted_int4, result);

                let _ = counter.fetch_add(1, Ordering::SeqCst);
            }
        };

        let plaintext = async {
            let sql = "SELECT id, plaintext FROM encrypted WHERE id = $1";
            let rows = client.query(sql, &[&plaintext_id]).await.expect("ok");

            assert!(rows.len() == 1);

            for row in rows {
                let result: String = row.get("plaintext");
                assert_eq!(plaintext_text, result);

                let _ = counter.fetch_add(1, Ordering::SeqCst);
            }
        };

        tokio::join!(text, plaintext, int2, int4);

        let count = counter.load(Ordering::SeqCst);
        assert_eq!(count, 4);
    }
}
