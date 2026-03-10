/// Tests that validate proxy connection isolation under load.
///
/// These tests verify that:
/// - Slow queries on one connection don't block other connections
/// - The proxy accepts new connections after client disconnect
/// - Concurrent connections under load remain responsive
/// - Blocked backend connections don't affect other proxy connections
#[cfg(test)]
mod tests {
    use crate::common::{connect_with_tls, get_database_port, PROXY};
    use std::time::Instant;
    use tokio::task::JoinSet;
    use tokio::time::{timeout, Duration};
    use tokio_postgres::SimpleQueryMessage;

    /// Advisory lock ID used in isolation tests. Arbitrary value — just needs to be
    /// unique across concurrently running test suites against the same database.
    const ADVISORY_LOCK_ID: i64 = 99_001;

    /// A slow query on one connection does not block other connections through the proxy.
    #[tokio::test]
    async fn slow_query_does_not_block_other_connections() {
        let result = timeout(Duration::from_secs(30), async {
            let client_a = connect_with_tls(PROXY).await;
            let client_b = connect_with_tls(PROXY).await;

            // Connection A: run a slow query
            let a_handle = tokio::spawn(async move {
                client_a.simple_query("SELECT pg_sleep(5)").await.unwrap();
            });

            // Brief pause to ensure A's query is in flight
            tokio::time::sleep(Duration::from_millis(200)).await;

            // Connection B: run a fast query, should complete promptly
            let start = Instant::now();
            let rows = client_b.simple_query("SELECT 1").await.unwrap();
            let elapsed = start.elapsed();

            assert!(!rows.is_empty(), "Expected result from SELECT 1");
            assert!(
                elapsed < Duration::from_secs(2),
                "Fast query took {elapsed:?}, expected < 2s — proxy may be blocking"
            );

            a_handle.await.unwrap();
        })
        .await;

        result.expect("Test timed out after 30s");
    }

    /// Proxy accepts new connections after a client disconnects.
    #[tokio::test]
    async fn proxy_accepts_new_connections_after_client_disconnect() {
        let result = timeout(Duration::from_secs(10), async {
            // First connection: query, then drop
            {
                let client = connect_with_tls(PROXY).await;
                let rows = client.simple_query("SELECT 1").await.unwrap();
                assert!(!rows.is_empty());
            }
            // Client dropped here

            // Brief pause
            tokio::time::sleep(Duration::from_millis(100)).await;

            // Second connection: should work fine
            let client = connect_with_tls(PROXY).await;
            let rows = client.simple_query("SELECT 1").await.unwrap();
            assert!(!rows.is_empty());
        })
        .await;

        result.expect("Test timed out after 10s");
    }

    /// Concurrent slow and fast connections: fast queries complete promptly under slow load.
    #[tokio::test]
    async fn concurrent_connections_under_slow_load() {
        let result = timeout(Duration::from_secs(30), async {
            let mut join_set = JoinSet::new();

            // 5 slow connections
            for _ in 0..5 {
                join_set.spawn(async {
                    let client = connect_with_tls(PROXY).await;
                    client.simple_query("SELECT pg_sleep(3)").await.unwrap();
                });
            }

            // Brief pause to let slow queries start
            tokio::time::sleep(Duration::from_millis(300)).await;

            // 5 fast connections, each should complete promptly
            for _ in 0..5 {
                join_set.spawn(async {
                    let start = Instant::now();
                    let client = connect_with_tls(PROXY).await;
                    let rows = client.simple_query("SELECT 1").await.unwrap();
                    let elapsed = start.elapsed();

                    assert!(!rows.is_empty());
                    assert!(
                        elapsed < Duration::from_secs(5),
                        "Fast query took {elapsed:?} under slow load, expected < 5s"
                    );
                });
            }

            while let Some(result) = join_set.join_next().await {
                result.unwrap();
            }
        })
        .await;

        result.expect("Test timed out after 30s");
    }

    /// An advisory-lock-blocked connection through the proxy does not block other proxy connections.
    ///
    /// Uses pg_locks polling to deterministically wait for client_b to be blocked on the
    /// advisory lock, rather than relying on a fixed sleep.
    #[tokio::test]
    async fn advisory_lock_blocked_connection_does_not_block_proxy() {
        let lock_query = format!("SELECT pg_advisory_lock({ADVISORY_LOCK_ID})");
        let unlock_query = format!("SELECT pg_advisory_unlock({ADVISORY_LOCK_ID})");

        let result = timeout(Duration::from_secs(30), async {
            // Connection A: hold an advisory lock (connect directly to PG to avoid proxy interference)
            let pg_port = get_database_port();
            let client_a = connect_with_tls(pg_port).await;
            client_a
                .simple_query(&lock_query)
                .await
                .unwrap();

            let b_lock_query = lock_query.clone();
            let b_unlock_query = unlock_query.clone();

            // Connection B: through proxy, attempt to acquire the same lock (will block)
            let b_handle = tokio::spawn(async move {
                let client_b = connect_with_tls(PROXY).await;
                // This will block until A releases the lock
                client_b
                    .simple_query(&b_lock_query)
                    .await
                    .unwrap();
                // Release after acquiring
                client_b
                    .simple_query(&b_unlock_query)
                    .await
                    .unwrap();
            });

            // Poll pg_locks until client_b is observed waiting for the advisory lock
            let poll_query = format!(
                "SELECT 1 FROM pg_locks WHERE locktype = 'advisory' AND NOT granted AND classid = 0 AND objid = {ADVISORY_LOCK_ID}"
            );
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let result = client_a.simple_query(&poll_query).await.unwrap();
                let has_waiting = result.iter().any(|m| matches!(m, SimpleQueryMessage::Row(_)));
                if has_waiting {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "Timed out waiting for client_b to be blocked on advisory lock"
                );
                tokio::time::sleep(Duration::from_millis(50)).await;
            }

            // Connection C: through proxy, should complete immediately despite B being blocked
            let start = Instant::now();
            let client_c = connect_with_tls(PROXY).await;
            let rows = client_c.simple_query("SELECT 1").await.unwrap();
            let elapsed = start.elapsed();

            assert!(!rows.is_empty());
            assert!(
                elapsed < Duration::from_secs(2),
                "Connection C took {elapsed:?}, expected < 2s — blocked connection may be affecting proxy"
            );

            // Release the lock so B can complete
            client_a
                .simple_query(&unlock_query)
                .await
                .unwrap();

            b_handle.await.unwrap();
        })
        .await;

        result.expect("Test timed out after 30s");
    }
}
