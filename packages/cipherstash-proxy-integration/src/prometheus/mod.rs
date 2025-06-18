// TODO: INSERT, see other tests
// TODO: HTTP,

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::common::{clear, insert, query, query_by, random_id, random_limited, trace};
    use once_cell::sync::Lazy;
    use regex::Regex;

    static P8S_SAMPLE_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"^(?P<name>\w+)(\{(?P<labels>[^}]+)\})?\s+(?P<value>\S+)(\s+(?P<timestamp>\S+))?",
        )
        .unwrap()
    });

    #[tokio::test]
    pub async fn totals() {
        trace();
        clear().await;

        let stats = get_stats().await;
        let baselines = [
            "cipherstash_proxy_statements_total",
            "cipherstash_proxy_rows_passthrough_total",
            "cipherstash_proxy_rows_total",
        ]
        .iter()
        .fold(HashMap::new(), |map, s| {
            let stat = s.to_string();
            map.insert(stat, stats.get(&stat).unwrap().clone()).unwrap();
            map
        });

        let sql = "SELECT 1";
        query::<i32>(sql).await;

        let tests = [
            ("cipherstash_proxy_statements_total", "1"),
            ("cipherstash_proxy_rows_passthrough_total", "1"),
            ("cipherstash_proxy_rows_total", "1"),
        ];

        let stats = get_stats().await;
        println!("{stats:#?}");
        for (stat, value) in tests {
            assert_stat(&baselines, &stats, stat, value);
        }

        let id = random_id();

        let encrypted_val = crate::value_for_type!(String, random_limited());
        let sql = "INSERT INTO encrypted (id, plaintext) VALUES ($1, $2)";
        insert(sql, &[&id, &encrypted_val]).await;

        let tests = [
            ("cipherstash_proxy_statements_total", "2"),
            ("cipherstash_proxy_statements_passthrough_total", "1"),
            ("cipherstash_proxy_encrypted_values_total", "1"),
        ];

        let stats = get_stats().await;
        //println!("{stats:#?}");
        for (stat, value) in tests {
            assert_stat(&baselines, &stats, stat, value);
        }

        let sql = "SELECT plaintext FROM encrypted WHERE id = $1";
        query_by::<String>(sql, &id).await;

        let tests = [
            ("cipherstash_proxy_statements_total", "3"),
            ("cipherstash_proxy_statements_encrypted_total", "2"),
            ("cipherstash_proxy_rows_encrypted_total", "1"),
        ];

        let stats = get_stats().await;
        for (stat, value) in tests {
            assert_stat(&baselines, &stats, stat, value);
        }
    }

    fn assert_stat(
        //baselines: &HashMap<String, String>,
        //stats: &HashMap<String, String>,
        baselines: &HashMap<&str, &str>,
        stats: &HashMap<&str, &str>,
        name: &str,
        expected: &str,
    ) {
        let stat = stats.get(&name.to_string()).unwrap().clone();
        assert_eq!(
            stat,
            expected.to_string(),
            "testing stat \"{}\" expecting to be \"{}\"",
            name,
            expected
        );
    }

    async fn get_stats<'a>() -> HashMap<&'a str, &'a str> {
        let mut stats = HashMap::new();
        let body = reqwest::get("http://localhost:9930")
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        let lines = body.lines().map(|line| P8S_SAMPLE_RE.captures(line));

        for line in lines {
            if let Some(ref caps) = line {
                if let (Some(ref name), Some(ref value)) = (caps.name("name"), caps.name("value")) {
                    stats.insert(name.to_owned().as_str(), value.to_owned().as_str());
                }
            }
        }
        stats
    }

    // Output sample:
    /*
        # HELP cipherstash_proxy_server_bytes_received_total Number of bytes CipherStash Proxy received from the PostgreSQL server
        # TYPE cipherstash_proxy_server_bytes_received_total counter
        cipherstash_proxy_server_bytes_received_total 1950

        # HELP cipherstash_proxy_clients_bytes_sent_total Number of bytes sent from CipherStash Proxy to clients
        # TYPE cipherstash_proxy_clients_bytes_sent_total counter
        cipherstash_proxy_clients_bytes_sent_total 1950

        # HELP cipherstash_proxy_clients_bytes_received_total Number of bytes received by CipherStash Proxy from clients
        # TYPE cipherstash_proxy_clients_bytes_received_total counter
        cipherstash_proxy_clients_bytes_received_total 406

        # HELP cipherstash_proxy_statements_total Total number of SQL statements processed by CipherStash Proxy
        # TYPE cipherstash_proxy_statements_total counter
        cipherstash_proxy_statements_total 6

        # HELP cipherstash_proxy_statements_passthrough_total Number of SQL statements that did not require encryption
        # TYPE cipherstash_proxy_statements_passthrough_total counter
        cipherstash_proxy_statements_passthrough_total 6

        # HELP cipherstash_proxy_server_bytes_sent_total Number of bytes CipherStash Proxy sent to the PostgreSQL server
        # TYPE cipherstash_proxy_server_bytes_sent_total counter
        cipherstash_proxy_server_bytes_sent_total 421

        # HELP cipherstash_proxy_clients_active_connections Current number of connections to CipherStash Proxy from clients
        # TYPE cipherstash_proxy_clients_active_connections gauge
        cipherstash_proxy_clients_active_connections 1

        # HELP cipherstash_proxy_statements_execution_duration_seconds Duration of time the proxied database spent executing SQL statements
        # TYPE cipherstash_proxy_statements_execution_duration_seconds summary
        cipherstash_proxy_statements_execution_duration_seconds{quantile="0"} 0.002213792
        cipherstash_proxy_statements_execution_duration_seconds{quantile="0.5"} 0.004472087294722821
        cipherstash_proxy_statements_execution_duration_seconds{quantile="0.9"} 0.004472087294722821
        cipherstash_proxy_statements_execution_duration_seconds{quantile="0.95"} 0.004472087294722821
        cipherstash_proxy_statements_execution_duration_seconds{quantile="0.99"} 0.004472087294722821
        cipherstash_proxy_statements_execution_duration_seconds{quantile="0.999"} 0.004472087294722821
        cipherstash_proxy_statements_execution_duration_seconds{quantile="1"} 0.016979625
        cipherstash_proxy_statements_execution_duration_seconds_sum 0.045398542
        cipherstash_proxy_statements_execution_duration_seconds_count 6

        # HELP cipherstash_proxy_statements_session_duration_seconds Duration of time CipherStash Proxy spent processing the statement including encryption, database execution and decryption.
        # TYPE cipherstash_proxy_statements_session_duration_seconds summary
        cipherstash_proxy_statements_session_duration_seconds{quantile="0"} 0.005194208
        cipherstash_proxy_statements_session_duration_seconds{quantile="0.5"} 0.017106444441250663
        cipherstash_proxy_statements_session_duration_seconds{quantile="0.9"} 0.017106444441250663
        cipherstash_proxy_statements_session_duration_seconds{quantile="0.95"} 0.017106444441250663
        cipherstash_proxy_statements_session_duration_seconds{quantile="0.99"} 0.017106444441250663
        cipherstash_proxy_statements_session_duration_seconds{quantile="0.999"} 0.017106444441250663
        cipherstash_proxy_statements_session_duration_seconds{quantile="1"} 0.022098959
        cipherstash_proxy_statements_session_duration_seconds_sum 0.09400696
        cipherstash_proxy_statements_session_duration_seconds_count 6
    */
}
