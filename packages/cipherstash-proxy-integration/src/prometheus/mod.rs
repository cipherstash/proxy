// TODO: INSERT, see other tests
// TODO: HTTP,

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::common::{clear, execute_query, query, query_by, random_id, random_limited, trace};
    use once_cell::sync::Lazy;
    use regex::Regex;

    static P8S_SAMPLE_RE: Lazy<Regex> = Lazy::new(|| {
        Regex::new(
            r"^(?P<name>\w+)(\{(?P<labels>[^}]+)\})?\s+(?P<value>\S+)(\s+(?P<timestamp>\S+))?",
        )
        .unwrap()
    });

    #[derive(Debug)]
    enum Stat {
        Int(i32),
        Float(f32),
    }

    impl PartialEq for Stat {
        fn eq(&self, other: &Self) -> bool {
            match (self, other) {
                (Stat::Int(x), Stat::Int(y)) => x == y,
                (Stat::Float(x), Stat::Float(y)) => format!("{:.5}", x) == format!("{:.5}", y),
                _ => false,
            }
        }
    }
    impl Eq for Stat {}

    impl Stat {
        fn as_i32(&self) -> Option<&i32> {
            if let Stat::Int(i) = &self {
                Some(i)
            } else {
                None
            }
        }
    }

    #[tokio::test]
    pub async fn totals() {
        trace();
        clear().await;

        let stats = get_stats().await;
        let baselines = [
            "cipherstash_proxy_statements_passthrough_total",
            "cipherstash_proxy_statements_encrypted_total",
            "cipherstash_proxy_statements_total",
            "cipherstash_proxy_rows_encrypted_total",
            "cipherstash_proxy_rows_passthrough_total",
            "cipherstash_proxy_rows_total",
        ]
        .into_iter()
        .fold(HashMap::new(), |mut map, s| {
            let value = stats.get(s);
            //assert!(value.is_some(), "Expected {} to be present in stats: {:#?}", s, stats.keys());
            map.insert(s.to_string(), value.unwrap_or(&Stat::Int(0)));
            map
        });

        let assert_int_stat = |stats: &HashMap<String, Stat>,
                               stat_name: &str,
                               expected: i32,
                               test_label: &str| {
            let baseline = &baselines.get(stat_name);
            let expected_with_baseline = baseline
                .and_then(|stat| stat.as_i32())
                .map(|i| Stat::Int(i + expected));
            let stat = stats.get(stat_name);
            assert_eq!(
                stat, expected_with_baseline.as_ref(),
                "for '{}': testing stat \"{}\" expecting to be \"{:?}\" after applying baseline of \"{:?}\"",
                test_label, stat_name, expected_with_baseline, baseline
            );
        };

        let sql = "SELECT 1";
        query::<i32>(sql).await;

        let tests = [
            ("cipherstash_proxy_statements_total", 1),
            ("cipherstash_proxy_rows_passthrough_total", 1),
            ("cipherstash_proxy_rows_total", 1),
        ];

        let stats = get_stats().await;
        println!("{stats:#?}");
        for (stat, expected) in tests {
            assert_int_stat(&stats, stat, expected, "initial SELECT 1 query");
        }

        let id = random_id();

        let encrypted_val = crate::value_for_type!(String, random_limited());
        let sql = "INSERT INTO encrypted (id, plaintext) VALUES ($1, $2)";
        execute_query(sql, &[&id, &encrypted_val]).await;

        let tests = [
            ("cipherstash_proxy_statements_total", 2),
            ("cipherstash_proxy_statements_passthrough_total", 1),
            ("cipherstash_proxy_encrypted_values_total", 1),
        ];

        let stats = get_stats().await;
        //println!("{stats:#?}");
        for (stat, expected) in tests {
            assert_int_stat(&stats, stat, expected, "INSERT encrypted");
        }

        let sql = "SELECT plaintext FROM encrypted WHERE id = $1";
        query_by::<String>(sql, &id).await;

        let tests = [
            ("cipherstash_proxy_statements_total", 3),
            ("cipherstash_proxy_statements_encrypted_total", 2),
            ("cipherstash_proxy_rows_encrypted_total", 1),
        ];

        let stats = get_stats().await;
        for (stat, expected) in tests {
            assert_int_stat(&stats, stat, expected, "SELECT plaintext");
        }
    }

    async fn get_stats() -> HashMap<String, Stat> {
        let mut stats = HashMap::new();
        let body = reqwest::get("http://localhost:9930")
            .await
            .unwrap()
            .text()
            .await
            .unwrap();
        // let body = "foo_bar 123\nbaz_qux 234".to_string();
        let lines = body.lines().map(|line| P8S_SAMPLE_RE.captures(line));

        for ref caps in lines.flatten() {
            if let (Some(ref name), Some(ref value)) = (caps.name("name"), caps.name("value")) {
                println!("{:?}", value);
                let stat = parse_stat(value.as_str());
                stats.insert(name.as_str().into(), stat);
            }
        }
        stats
    }

    fn parse_stat(value: &str) -> Stat {
        // This is awful. Anyway!
        if value.contains(".") {
            Stat::Float(str::parse::<f32>(value).unwrap())
        } else {
            Stat::Int(str::parse::<i32>(value).unwrap())
        }
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
