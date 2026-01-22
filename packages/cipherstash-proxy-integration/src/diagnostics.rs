#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, PROXY, PROXY_METRICS_PORT};

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
        let body = fetch_metrics_with_retry(5, 200).await;

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
}
