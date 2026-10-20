#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, random_id, trace, PROXY};
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::task::Poll;
    use std::time::Duration;
    use tokio_postgres::SimpleQueryMessage;

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

        let client = connect_with_tls(*PROXY).await;

        let counter = AtomicUsize::new(0);

        let text_id = random_id();
        let encrypted_text = "hello@cipherstash.com";

        let text = async {
            let sql = "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)";
            client
                .query(sql, &[&text_id, &encrypted_text])
                .await
                .unwrap();

            let _ = counter.fetch_add(1, Ordering::SeqCst);
        };

        let int2_id = random_id();
        let encrypted_int2: i16 = 16;

        let int2 = async {
            let sql = "INSERT INTO encrypted (id, encrypted_int2) VALUES ($1, $2)";
            client
                .query(sql, &[&int2_id, &encrypted_int2])
                .await
                .unwrap();

            let _ = counter.fetch_add(1, Ordering::SeqCst);
        };

        let int4_id = random_id();
        let encrypted_int4: i32 = 32;

        let int4 = async {
            let sql = "INSERT INTO encrypted (id, encrypted_int4) VALUES ($1, $2)";
            client
                .query(sql, &[&int4_id, &encrypted_int4])
                .await
                .unwrap();

            let _ = counter.fetch_add(1, Ordering::SeqCst);
        };

        let plaintext_id = random_id();
        let plaintext_text = "blahvtha";

        let plaintext = async {
            let sql = "INSERT INTO encrypted (id, plaintext) VALUES ($1, $2)";
            client
                .query(sql, &[&plaintext_id, &plaintext_text])
                .await
                .unwrap();

            let _ = counter.fetch_add(1, Ordering::SeqCst);
        };

        tokio::join!(text, plaintext, int2, int4);

        let count = counter.load(Ordering::SeqCst);
        assert_eq!(count, 4);

        let counter = AtomicUsize::new(0);

        let text = async {
            let sql = "SELECT id, encrypted_text FROM encrypted WHERE id = $1";
            let rows = client.query(sql, &[&text_id]).await.unwrap();

            assert_eq!(rows.len(), 1);

            for row in rows {
                let result: String = row.get("encrypted_text");
                assert_eq!(encrypted_text, result);

                let _ = counter.fetch_add(1, Ordering::SeqCst);
            }
        };

        let int2 = async {
            let sql = "SELECT id, encrypted_int2 FROM encrypted WHERE id = $1";
            let rows = client.query(sql, &[&int2_id]).await.unwrap();

            assert_eq!(rows.len(), 1);

            for row in rows {
                let result: i16 = row.get("encrypted_int2");
                assert_eq!(encrypted_int2, result);

                let _ = counter.fetch_add(1, Ordering::SeqCst);
            }
        };

        let int4 = async {
            let sql = "SELECT id, encrypted_int4 FROM encrypted WHERE id = $1";
            let rows = client.query(sql, &[&int4_id]).await.unwrap();

            assert_eq!(rows.len(), 1);

            for row in rows {
                let result: i32 = row.get("encrypted_int4");
                assert_eq!(encrypted_int4, result);

                let _ = counter.fetch_add(1, Ordering::SeqCst);
            }
        };

        let plaintext = async {
            let sql = "SELECT id, plaintext FROM encrypted WHERE id = $1";
            let rows = client.query(sql, &[&plaintext_id]).await.unwrap();

            assert_eq!(rows.len(), 1);

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

    /// More queries than Proxy's 256-message pipeline bound, sent before any
    /// response is read.
    const OVER_PIPELINE_BOUND: usize = 300;

    ///
    /// Test that a client pipeline deeper than Proxy's bound still completes
    ///
    /// The first query sleeps so responses back up and Proxy's pipeline fills.
    /// Proxy must then drain responses and retry the rejected message, including
    /// a settings command and a query that depends on it, rather than stall.
    ///
    #[tokio::test]
    async fn pipeline_deeper_than_proxy_bound_completes() {
        trace();

        clear().await;

        let client = connect_with_tls(*PROXY).await;

        let id = random_id();
        let encrypted_text = "deep-pipeline@cipherstash.com";
        client
            .query(
                "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
                &[&id, &encrypted_text],
            )
            .await
            .unwrap();

        let mut sql = vec!["SELECT pg_sleep(0.5)".to_string()];
        sql.extend((1..OVER_PIPELINE_BOUND - 2).map(|n| format!("SELECT {n}")));
        sql.push("SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = false".to_string());
        sql.push(format!(
            "SELECT encrypted_text FROM encrypted WHERE id = {id}"
        ));
        assert_eq!(sql.len(), OVER_PIPELINE_BOUND);

        type Pending<'a> = Pin<
            Box<dyn Future<Output = Result<Vec<SimpleQueryMessage>, tokio_postgres::Error>> + 'a>,
        >;
        let mut pending: Vec<Pending<'_>> = sql
            .iter()
            .map(|sql| Box::pin(client.simple_query(sql)) as Pending<'_>)
            .collect();
        let mut results = (0..pending.len()).map(|_| None).collect::<Vec<_>>();

        // Poll every query in order on each wake, so all requests are written
        // in order before any response arrives.
        let all = std::future::poll_fn(|cx| {
            let mut done = true;
            for (result, query) in results.iter_mut().zip(pending.iter_mut()) {
                if result.is_none() {
                    match query.as_mut().poll(cx) {
                        Poll::Ready(outcome) => *result = Some(outcome),
                        Poll::Pending => done = false,
                    }
                }
            }
            if done {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        });
        tokio::time::timeout(Duration::from_secs(30), all)
            .await
            .unwrap();

        let results = results
            .into_iter()
            .map(|result| result.unwrap().unwrap())
            .collect::<Vec<_>>();

        let values = results
            .last()
            .unwrap()
            .iter()
            .filter_map(|message| match message {
                SimpleQueryMessage::Row(row) => Some(row.get(0).map(str::to_owned)),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(values, vec![Some(encrypted_text.to_string())]);
    }
}
