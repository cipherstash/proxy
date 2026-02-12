#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, random_id, random_string, trace, PROXY};
    use std::time::Instant;
    use tokio::task::JoinSet;

    const CONNECTIONS: usize = 10;
    const INSERTS_PER_CONNECTION: usize = 5;

    /// Perform N encrypted inserts on the given client, returning the wall-clock duration.
    async fn do_encrypted_inserts(
        client: &tokio_postgres::Client,
        n: usize,
    ) -> std::time::Duration {
        let start = Instant::now();
        for _ in 0..n {
            let id = random_id();
            let val = random_string();
            client
                .query(
                    "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
                    &[&id, &val],
                )
                .await
                .unwrap();
        }
        start.elapsed()
    }

    /// Measures whether concurrent encrypted inserts scale better than sequential.
    ///
    /// Sequential: 10 serial connections, each doing 5 encrypted inserts.
    /// Concurrent: 10 parallel connections, each doing 5 encrypted inserts.
    ///
    /// With shared mutex contention, concurrent will be ~same or slower than sequential.
    /// After per-connection cipher fix, concurrent should be significantly faster.
    #[tokio::test]
    async fn concurrent_encrypted_inserts_measure_scaling() {
        trace();
        clear().await;

        // --- Sequential phase ---
        let seq_start = Instant::now();
        for _ in 0..CONNECTIONS {
            let client = connect_with_tls(PROXY).await;
            do_encrypted_inserts(&client, INSERTS_PER_CONNECTION).await;
        }
        let sequential_duration = seq_start.elapsed();

        clear().await;

        // --- Concurrent phase ---
        let conc_start = Instant::now();
        let mut join_set = JoinSet::new();

        for _ in 0..CONNECTIONS {
            join_set.spawn(async move {
                let client = connect_with_tls(PROXY).await;
                do_encrypted_inserts(&client, INSERTS_PER_CONNECTION).await;
            });
        }

        while let Some(result) = join_set.join_next().await {
            result.unwrap();
        }
        let concurrent_duration = conc_start.elapsed();

        // --- Diagnostics ---
        let scaling_factor = concurrent_duration.as_secs_f64() / sequential_duration.as_secs_f64();

        eprintln!("=== concurrent_encrypted_inserts_measure_scaling ===");
        eprintln!(
            "  Sequential ({CONNECTIONS} serial connections x {INSERTS_PER_CONNECTION} inserts): {:.3}s",
            sequential_duration.as_secs_f64()
        );
        eprintln!(
            "  Concurrent ({CONNECTIONS} parallel connections x {INSERTS_PER_CONNECTION} inserts): {:.3}s",
            concurrent_duration.as_secs_f64()
        );
        eprintln!("  Scaling factor (concurrent / sequential): {scaling_factor:.3}");
        eprintln!("  (After fix: expect scaling_factor < 0.5)");
        eprintln!("====================================================");

        assert!(
            scaling_factor < 0.5,
            "Expected concurrent to be at least 2x faster than sequential, got scaling_factor={scaling_factor:.3}"
        );
    }

    /// Measures whether per-connection latency increases under concurrency.
    ///
    /// Solo: 1 connection doing 5 encrypted inserts.
    /// Concurrent: 10 connections each doing 5 encrypted inserts, measuring per-connection avg.
    ///
    /// With shared mutex contention, per-connection latency will increase significantly.
    /// After per-connection cipher fix, latency should remain stable.
    #[tokio::test]
    async fn per_connection_latency_increases_with_concurrency() {
        trace();
        clear().await;

        // --- Solo phase ---
        let solo_client = connect_with_tls(PROXY).await;
        let solo_duration = do_encrypted_inserts(&solo_client, INSERTS_PER_CONNECTION).await;

        clear().await;

        // --- Concurrent phase ---
        let mut join_set = JoinSet::new();

        for _ in 0..CONNECTIONS {
            join_set.spawn(async move {
                let client = connect_with_tls(PROXY).await;
                do_encrypted_inserts(&client, INSERTS_PER_CONNECTION).await
            });
        }

        let mut concurrent_durations = Vec::with_capacity(CONNECTIONS);
        while let Some(result) = join_set.join_next().await {
            concurrent_durations.push(result.unwrap());
        }

        let avg_concurrent = concurrent_durations
            .iter()
            .map(|d| d.as_secs_f64())
            .sum::<f64>()
            / concurrent_durations.len() as f64;

        let max_concurrent = concurrent_durations
            .iter()
            .map(|d| d.as_secs_f64())
            .fold(0.0_f64, f64::max);

        // --- Diagnostics ---
        let latency_multiplier = avg_concurrent / solo_duration.as_secs_f64();

        eprintln!("=== per_connection_latency_increases_with_concurrency ===");
        eprintln!(
            "  Solo (1 connection x {INSERTS_PER_CONNECTION} inserts): {:.3}s",
            solo_duration.as_secs_f64()
        );
        eprintln!(
            "  Concurrent avg ({CONNECTIONS} connections x {INSERTS_PER_CONNECTION} inserts): {avg_concurrent:.3}s",
        );
        eprintln!("  Concurrent max: {max_concurrent:.3}s");
        eprintln!("  Latency multiplier (avg_concurrent / solo): {latency_multiplier:.3}");
        eprintln!("  (After fix: expect latency_multiplier < 2.0)");
        eprintln!("=========================================================");

        assert!(
            latency_multiplier < 2.0,
            "Expected per-connection latency to stay stable under concurrency, got multiplier={latency_multiplier:.3}"
        );
    }

    /// Verifies that a slow connection does not block other connections.
    ///
    /// Connection A: encrypted insert then pg_sleep(0.5).
    /// Connection B (spawned 50ms after A): 3 encrypted inserts, measure total time.
    ///
    /// With shared mutex contention, B may be blocked while A holds a lock during sleep.
    /// After per-connection cipher fix, B should complete independently of A's sleep.
    #[tokio::test]
    async fn slow_connection_does_not_block_other_connections() {
        trace();
        clear().await;

        // Connection A: insert then sleep
        let a_handle = tokio::spawn(async move {
            let client = connect_with_tls(PROXY).await;
            let id = random_id();
            let val = random_string();
            client
                .query(
                    "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
                    &[&id, &val],
                )
                .await
                .unwrap();

            // Hold this connection busy with a sleep
            client.simple_query("SELECT pg_sleep(0.5)").await.unwrap();
        });

        // Small delay so A is likely in-flight before B starts
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Connection B: 3 encrypted inserts, timed
        let b_handle = tokio::spawn(async move {
            let client = connect_with_tls(PROXY).await;
            let start = Instant::now();
            for _ in 0..3 {
                let id = random_id();
                let val = random_string();
                client
                    .query(
                        "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
                        &[&id, &val],
                    )
                    .await
                    .unwrap();
            }
            start.elapsed()
        });

        // Wait for both
        let b_duration = b_handle.await.unwrap();
        a_handle.await.unwrap();

        // --- Diagnostics ---
        eprintln!("=== slow_connection_does_not_block_other_connections ===");
        eprintln!(
            "  Connection B (3 encrypted inserts while A sleeps): {:.3}s",
            b_duration.as_secs_f64()
        );
        eprintln!("  (After fix: expect B completes well under 0.5s, independent of A's sleep)");
        eprintln!("========================================================");

        assert!(
            b_duration.as_secs_f64() < 0.5,
            "Connection B should not be blocked by A's sleep, took {:.3}s",
            b_duration.as_secs_f64()
        );
    }
}
