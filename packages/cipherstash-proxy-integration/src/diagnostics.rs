#[cfg(test)]
mod tests {
    use crate::common::{clear, connect_with_tls, PROXY};

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

        // Give the metrics some time to be written
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;

        // Fetch metrics from the /metrics endpoint
        let body = reqwest::get("http://localhost:9930/metrics")
            .await
            .unwrap()
            .text()
            .await
            .unwrap();

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
