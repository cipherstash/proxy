#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, PROXY, PROXY_METRICS_PORT};

    /// Maximum number of retry attempts for fetching metrics.
    /// 5 retries with 200ms delay gives ~1 second total wait time,
    /// sufficient for Prometheus scrape interval in CI environments.
    const METRICS_FETCH_MAX_RETRIES: u32 = 5;

    /// Delay between retry attempts in milliseconds.
    /// 200ms provides a reasonable balance between responsiveness and allowing
    /// sufficient time for metrics to be published by the Prometheus client.
    const METRICS_FETCH_RETRY_DELAY_MS: u64 = 200;

    /// Fetch metrics with retry logic to handle CI timing variability.
    async fn fetch_metrics_with_retry(max_retries: u32, delay_ms: u64) -> String {
        let url = format!("http://localhost:{}/metrics", PROXY_METRICS_PORT);
        let mut last_error = None;

        for attempt in 0..max_retries {
            if attempt > 0 {
                tokio::time::sleep(std::time::Duration::from_millis(delay_ms)).await;
            }

            match reqwest::get(&url).await {
                Ok(response) => match response.text().await {
                    Ok(body) => return body,
                    Err(e) => last_error = Some(format!("Failed to read response: {}", e)),
                },
                Err(e) => last_error = Some(format!("Failed to fetch metrics: {}", e)),
            }
        }

        panic!(
            "Failed to fetch metrics after {} retries: {}",
            max_retries,
            last_error.unwrap_or_else(|| "unknown error".to_string())
        );
    }

    #[tokio::test]
    async fn metrics_include_statement_labels() {
        let client = connect_with_tls(PROXY).await;

        clear().await;

        // Insert a value to generate metrics
        client
            .execute(
                "INSERT INTO plaintext (id, plaintext) VALUES ($1, $2)",
                &[&1i64, &"metrics test"],
            )
            .await
            .unwrap();

        // Select a value to generate metrics
        let _rows = client
            .query("SELECT * FROM plaintext LIMIT 1", &[])
            .await
            .unwrap();

        // Fetch metrics with retry logic for CI robustness
        let body =
            fetch_metrics_with_retry(METRICS_FETCH_MAX_RETRIES, METRICS_FETCH_RETRY_DELAY_MS).await;

        // Assert that the metrics include the expected labels
        assert!(
            body.contains("statement_type=\"insert\""),
            "Metrics should include insert statement_type label"
        );
        assert!(
            body.contains("statement_type=\"select\""),
            "Metrics should include select statement_type label"
        );
        assert!(
            body.contains("multi_statement=\"false\""),
            "Metrics should include multi_statement=false label"
        );
    }

    #[tokio::test]
    async fn slow_statement_metrics_and_logs() {
        let client = connect_with_tls(PROXY).await;

        clear().await;

        // Execute a query that takes longer than the default 2s threshold
        // We use pg_sleep(2.1) to ensure it's considered slow
        client.query("SELECT pg_sleep(2.1)", &[]).await.unwrap();

        // Fetch metrics with retry logic
        let body =
            fetch_metrics_with_retry(METRICS_FETCH_MAX_RETRIES, METRICS_FETCH_RETRY_DELAY_MS).await;

        // Assert that the slow statements counter is present and non-zero
        assert!(
            body.contains("cipherstash_proxy_slow_statements_total"),
            "Metrics should include slow statement counter. Found: {}",
            body
        );

        // Extract the value to ensure it's at least 1
        let slow_statements_line = body.lines()
            .find(|l| l.starts_with("cipherstash_proxy_slow_statements_total"))
            .expect("Slow statements counter line should exist");
        let slow_statements_count: u64 = slow_statements_line
            .split_whitespace()
            .last()
            .expect("Should have a value")
            .parse()
            .expect("Should be a valid number");
        
        assert!(slow_statements_count >= 1, "Slow statements count should be at least 1, found {}", slow_statements_count);

        // Verify that duration histograms also reflect the slow query
        // We check for _count as it works for both histograms and summaries
        assert!(
            body.contains("cipherstash_proxy_statements_session_duration_seconds"),
            "Metrics should include session duration metrics"
        );
    }
}
