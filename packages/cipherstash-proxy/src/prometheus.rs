use crate::error::Error;
use crate::log::DEVELOPMENT;
use metrics::{describe_counter, describe_gauge, describe_histogram, gauge, Unit};
use metrics_exporter_prometheus::PrometheusBuilder;
use std::net::SocketAddr;
use tracing::{debug, info};

// See https://prometheus.io/docs/practices/naming/
pub const ENCRYPTED_VALUES_TOTAL: &str = "cipherstash_proxy_encrypted_values_total";
pub const ENCRYPTION_ERROR_TOTAL: &str = "cipherstash_proxy_encryption_error_total";
pub const ENCRYPTION_DURATION_SECONDS: &str = "cipherstash_proxy_encryption_duration_seconds";

pub const DECRYPTED_VALUES_TOTAL: &str = "cipherstash_proxy_decrypted_values_total";
pub const DECRYPTION_ERROR_TOTAL: &str = "cipherstash_proxy_decryption_error_total";
pub const DECRYPTION_DURATION_SECONDS: &str = "cipherstash_proxy_decryption_duration_seconds";

pub const STATEMENTS_TOTAL: &str = "cipherstash_proxy_statements_total";
pub const STATEMENTS_ENCRYPTED_TOTAL: &str = "cipherstash_proxy_statements_encrypted_total";
pub const STATEMENTS_PASSTHROUGH_TOTAL: &str = "cipherstash_proxy_statements_passthrough_total";
pub const STATEMENT_UNMAPPABLE_TOTAL: &str = "cipherstash_proxy_statements_unmappable_total";
pub const STATEMENT_DURATION_SECONDS: &str = "cipherstash_proxy_statements_duration_seconds";

pub const ROWS_TOTAL: &str = "cipherstash_proxy_rows_total";
pub const ROWS_ENCRYPTED_TOTAL: &str = "cipherstash_proxy_rows_encrypted_total";
pub const ROWS_PASSTHROUGH_TOTAL: &str = "cipherstash_proxy_rows_passthrough_total";

pub const CLIENTS_ACTIVE_CONNECTIONS: &str = "cipherstash_proxy_clients_active_connections";
pub const CLIENTS_BYTES_SENT_TOTAL: &str = "cipherstash_proxy_clients_bytes_sent_total";
pub const CLIENTS_BYTES_RECEIVED_TOTAL: &str = "cipherstash_proxy_clients_bytes_received_total";
pub const SERVER_BYTES_SENT_TOTAL: &str = "cipherstash_proxy_server_bytes_sent_total";
pub const SERVER_BYTES_RECEIVED_TOTAL: &str = "cipherstash_proxy_server_bytes_received_total";

pub fn start(host: String, port: u16) -> Result<(), Error> {
    let address = format!("{}:{}", host, port);
    let socket_address: SocketAddr = address.parse().unwrap();

    debug!(target: DEVELOPMENT, msg = "Starting Prometheus exporter", port);

    PrometheusBuilder::new()
        .with_http_listener(socket_address)
        .install()?;

    describe_counter!(ENCRYPTED_VALUES_TOTAL, "Number of encrypted values");
    describe_counter!(ENCRYPTION_ERROR_TOTAL, "Number of encryption errors");
    describe_histogram!(
        ENCRYPTION_DURATION_SECONDS,
        Unit::Seconds,
        "Duration of encryption operations"
    );
    describe_counter!(DECRYPTED_VALUES_TOTAL, "Number of decrypted values");
    describe_counter!(DECRYPTION_ERROR_TOTAL, "Number of decryption errors");
    describe_histogram!(
        DECRYPTION_DURATION_SECONDS,
        Unit::Seconds,
        "Duration of decryption operations"
    );

    describe_counter!(STATEMENTS_TOTAL, "Total number of SQL statements");
    describe_counter!(
        STATEMENTS_ENCRYPTED_TOTAL,
        "Number of encrypted SQL statements"
    );
    describe_counter!(
        STATEMENTS_PASSTHROUGH_TOTAL,
        "Number of passthrough (non-encrypted) SQL statements"
    );
    describe_histogram!(
        STATEMENT_DURATION_SECONDS,
        Unit::Seconds,
        "Duration of statement execution"
    );

    describe_counter!(ROWS_TOTAL, "Number of rows returned");
    describe_counter!(ROWS_ENCRYPTED_TOTAL, "Number of encrypted rows returned");
    describe_counter!(
        ROWS_PASSTHROUGH_TOTAL,
        "Number of passthrough (non-encrypted) rows returned"
    );

    describe_gauge!(
        CLIENTS_ACTIVE_CONNECTIONS,
        "Current number of client connections"
    );
    describe_counter!(
        CLIENTS_BYTES_SENT_TOTAL,
        "Number of bytes sent to the client"
    );
    describe_counter!(
        CLIENTS_BYTES_RECEIVED_TOTAL,
        "Number of bytes received from the client"
    );

    describe_counter!(
        SERVER_BYTES_SENT_TOTAL,
        "Number of bytes sent to the server"
    );
    describe_counter!(
        SERVER_BYTES_RECEIVED_TOTAL,
        "Number of bytes received from the server"
    );

    // Prometheus endpoint is empty on startup and looks like an error
    // Explicitly set count to zero
    gauge!(CLIENTS_ACTIVE_CONNECTIONS).set(0);

    info!(msg = "Prometheus exporter started", port);
    Ok(())
}
