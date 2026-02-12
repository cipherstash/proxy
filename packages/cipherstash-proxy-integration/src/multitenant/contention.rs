/// Tests that validate mutex contention across concurrent multitenant connections.
///
/// All proxy connections share a single `Arc<ZerokmsClient>` which internally holds two mutexes:
/// - `Mutex<ChaChaRng>` in `ViturClient` — serializes IV generation during every encrypt call
/// - `Arc<Mutex<ServiceCredentials>>` in `AutoRefresh` — serializes token retrieval
///
/// In multitenant deployments, different tenants (keysets) encrypting concurrently all contend
/// on these same mutexes. These tests prove that contention exists and will validate the
/// per-connection cipher fix.
///
/// IMPORTANT: These tests require `CS_DEFAULT_KEYSET_ID` to be unset and tenant keyset
/// env vars to be set. They run in the multitenant integration test phase.
#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, random_id, random_string, trace, PROXY};
    use std::time::Instant;
    use tokio::task::JoinSet;

    /// Number of tenant connections per test phase.
    const TENANTS_PER_BATCH: usize = 10;

    /// Number of encrypted inserts each tenant performs.
    const INSERTS_PER_TENANT: usize = 50;

    /// Read tenant keyset IDs from environment, cycling through the 3 available keysets.
    fn tenant_keyset_ids(count: usize) -> Vec<String> {
        let keysets = [
            std::env::var("CS_TENANT_KEYSET_ID_1").unwrap(),
            std::env::var("CS_TENANT_KEYSET_ID_2").unwrap(),
            std::env::var("CS_TENANT_KEYSET_ID_3").unwrap(),
        ];
        (0..count)
            .map(|i| keysets[i % keysets.len()].clone())
            .collect()
    }

    /// Establish a connection and set the keyset for a tenant.
    /// Returns the ready-to-use client (connection setup is excluded from timing).
    async fn connect_as_tenant(keyset_id: &str) -> tokio_postgres::Client {
        let client = connect_with_tls(PROXY).await;
        let sql = format!("SET CIPHERSTASH.KEYSET_ID = '{keyset_id}'");
        client.query(&sql, &[]).await.unwrap();
        client
    }

    /// Perform N encrypted inserts on an already-connected client.
    /// Returns the wall-clock duration of the insert phase only.
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

    /// Measures whether concurrent multitenant encrypted inserts scale better than sequential.
    ///
    /// Sequential: 10 tenants in series, each doing 50 encrypted inserts.
    /// Concurrent: 10 tenants in parallel, each doing 50 encrypted inserts.
    ///
    /// Connection setup and keyset configuration happen before timing starts.
    /// Only the encrypt+insert phase is measured.
    ///
    /// With shared mutex contention, concurrent wall-clock will be ~same as sequential.
    /// After per-connection cipher fix, concurrent should be significantly faster.
    #[tokio::test]
    async fn multitenant_concurrent_encrypted_inserts_measure_scaling() {
        trace();
        clear().await;

        let keyset_ids = tenant_keyset_ids(TENANTS_PER_BATCH);

        // --- Sequential phase: establish all connections first, then measure inserts ---
        let mut seq_clients = Vec::with_capacity(TENANTS_PER_BATCH);
        for keyset_id in &keyset_ids {
            seq_clients.push(connect_as_tenant(keyset_id).await);
        }

        let seq_start = Instant::now();
        for client in &seq_clients {
            do_encrypted_inserts(client, INSERTS_PER_TENANT).await;
        }
        let sequential_duration = seq_start.elapsed();
        drop(seq_clients);

        clear().await;

        // --- Concurrent phase: establish all connections first, then measure inserts ---
        let mut conc_clients = Vec::with_capacity(TENANTS_PER_BATCH);
        for keyset_id in &keyset_ids {
            conc_clients.push(connect_as_tenant(keyset_id).await);
        }

        let conc_start = Instant::now();
        let mut join_set = JoinSet::new();

        for client in conc_clients {
            join_set.spawn(async move {
                do_encrypted_inserts(&client, INSERTS_PER_TENANT).await;
            });
        }

        while let Some(result) = join_set.join_next().await {
            result.unwrap();
        }
        let concurrent_duration = conc_start.elapsed();

        // --- Diagnostics ---
        let scaling_factor = concurrent_duration.as_secs_f64() / sequential_duration.as_secs_f64();

        eprintln!("=== multitenant_concurrent_encrypted_inserts_measure_scaling ===");
        eprintln!(
            "  Sequential ({TENANTS_PER_BATCH} tenants x {INSERTS_PER_TENANT} inserts): {:.3}s",
            sequential_duration.as_secs_f64()
        );
        eprintln!(
            "  Concurrent ({TENANTS_PER_BATCH} tenants x {INSERTS_PER_TENANT} inserts): {:.3}s",
            concurrent_duration.as_secs_f64()
        );
        eprintln!("  Scaling factor (concurrent / sequential): {scaling_factor:.3}");
        eprintln!("  (After fix: expect scaling_factor < 0.5)");
        eprintln!("================================================================");

        assert!(
            scaling_factor < 0.5,
            "Expected concurrent to be at least 2x faster than sequential, got scaling_factor={scaling_factor:.3}"
        );
    }

    /// Measures whether per-tenant latency increases under concurrent multitenant load.
    ///
    /// Solo: 1 tenant doing 50 encrypted inserts alone.
    /// Concurrent: 10 tenants each doing 50 encrypted inserts, measuring per-tenant duration.
    ///
    /// Connection setup is excluded from timing.
    ///
    /// With shared mutex contention, per-tenant latency will increase significantly.
    /// After per-connection cipher fix, latency should remain stable.
    #[tokio::test]
    async fn multitenant_per_connection_latency_increases_with_concurrency() {
        trace();
        clear().await;

        let keyset_ids = tenant_keyset_ids(TENANTS_PER_BATCH);

        // --- Solo phase ---
        let solo_client = connect_as_tenant(&keyset_ids[0]).await;
        let solo_duration = do_encrypted_inserts(&solo_client, INSERTS_PER_TENANT).await;
        drop(solo_client);

        clear().await;

        // --- Concurrent phase: establish all connections, then measure ---
        let mut clients = Vec::with_capacity(TENANTS_PER_BATCH);
        for keyset_id in &keyset_ids {
            clients.push(connect_as_tenant(keyset_id).await);
        }

        let mut join_set = JoinSet::new();
        for client in clients {
            join_set.spawn(async move { do_encrypted_inserts(&client, INSERTS_PER_TENANT).await });
        }

        let mut concurrent_durations = Vec::with_capacity(TENANTS_PER_BATCH);
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

        eprintln!("=== multitenant_per_connection_latency_increases_with_concurrency ===");
        eprintln!(
            "  Solo (1 tenant x {INSERTS_PER_TENANT} inserts): {:.3}s",
            solo_duration.as_secs_f64()
        );
        eprintln!(
            "  Concurrent avg ({TENANTS_PER_BATCH} tenants x {INSERTS_PER_TENANT} inserts): {avg_concurrent:.3}s",
        );
        eprintln!("  Concurrent max: {max_concurrent:.3}s");
        eprintln!("  Latency multiplier (avg_concurrent / solo): {latency_multiplier:.3}");
        eprintln!("  (After fix: expect latency_multiplier < 2.0)");
        eprintln!("=====================================================================");

        assert!(
            latency_multiplier < 2.0,
            "Expected per-tenant latency to stay stable under concurrency, got multiplier={latency_multiplier:.3}"
        );
    }

    /// Verifies that a slow tenant connection does not block other tenants.
    ///
    /// Tenant A: encrypted insert then pg_sleep(0.5).
    /// Tenant B (different keyset, spawned 50ms later): 10 encrypted inserts, timed.
    ///
    /// Connection setup is excluded from timing.
    ///
    /// With shared mutex contention, B may be blocked while A holds a lock.
    /// After per-connection cipher fix, B should complete independently of A's sleep.
    #[tokio::test]
    async fn multitenant_slow_connection_does_not_block_other_tenants() {
        trace();
        clear().await;

        let keyset_ids = tenant_keyset_ids(2);

        // Establish both connections before timing
        let client_a = connect_as_tenant(&keyset_ids[0]).await;
        let client_b = connect_as_tenant(&keyset_ids[1]).await;

        // Tenant A: encrypted insert then sleep
        let a_handle = tokio::spawn(async move {
            let id = random_id();
            let val = random_string();
            client_a
                .query(
                    "INSERT INTO encrypted (id, encrypted_text) VALUES ($1, $2)",
                    &[&id, &val],
                )
                .await
                .unwrap();

            // Hold this connection busy with a sleep
            client_a.simple_query("SELECT pg_sleep(0.5)").await.unwrap();
        });

        // Small delay so A is likely in-flight before B starts
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;

        // Tenant B: encrypted inserts, timed
        let b_handle = tokio::spawn(async move {
            let start = Instant::now();
            for _ in 0..10 {
                let id = random_id();
                let val = random_string();
                client_b
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
        eprintln!("=== multitenant_slow_connection_does_not_block_other_tenants ===");
        eprintln!(
            "  Tenant B (10 encrypted inserts while Tenant A sleeps): {:.3}s",
            b_duration.as_secs_f64()
        );
        eprintln!("  (After fix: expect B completes well under 0.5s, independent of A's sleep)");
        eprintln!("=================================================================");

        assert!(
            b_duration.as_secs_f64() < 0.5,
            "Tenant B should not be blocked by Tenant A's sleep, took {:.3}s",
            b_duration.as_secs_f64()
        );
    }
}
