use crate::error::Error;
use metrics::{describe_counter, describe_gauge, describe_histogram, gauge, Unit};
use metrics_exporter_prometheus::PrometheusBuilder;
use std::net::SocketAddr;
use tracing::info;

pub const ENCRYPTION_COUNT: &str = "encryption_count";
pub const ENCRYPTION_ERROR_COUNT: &str = "encryption_error_count";
pub const ENCRYPTION_DURATION: &str = "encryption_duration";
pub const DECRYPTION_COUNT: &str = "decryption_count";
pub const DECRYPTION_ERROR_COUNT: &str = "decryption_error_count";
pub const DECRYPTION_DURATION: &str = "decryption_duration";
pub const STATEMENT_TOTAL_COUNT: &str = "statement_total_count";
pub const STATEMENT_ENCRYPTED_COUNT: &str = "statement_encrypted_count";
pub const STATEMENT_PASSTHROUGH_COUNT: &str = "statement_passthrough_count";
pub const STATEMENT_UNMAPPABLE_COUNT: &str = "statement_unmappable_count";
pub const STATEMENT_DURATION: &str = "statement_duration";
pub const ROW_TOTAL_COUNT: &str = "row_total_count";
pub const ROW_ENCRYPTED_COUNT: &str = "row_encrypted_count";
pub const ROW_PASSTHROUGH_COUNT: &str = "row_passthrough_count";
pub const CLIENT_CONNECTION_COUNT: &str = "client_connection_count";
pub const CLIENT_BYTES_SENT: &str = "client_bytes_sent";
pub const CLIENT_BYTES_RECEIVED: &str = "client_bytes_received";
pub const SERVER_BYTES_SENT: &str = "server_bytes_sent";
pub const SERVER_BYTES_RECEIVED: &str = "server_bytes_received";

pub fn start(host: String, port: u16) -> Result<(), Error> {
    let address = format!("{}:{}", host, port);
    let socket_address: SocketAddr = address.parse().unwrap();

    PrometheusBuilder::new()
        .with_http_listener(socket_address)
        .install()?;

    describe_counter!(ENCRYPTION_COUNT, "Number of encryption actions.");
    describe_counter!(ENCRYPTION_ERROR_COUNT, "Number of encryption errors.");
    describe_histogram!(
        ENCRYPTION_DURATION,
        Unit::Milliseconds,
        "Duration of encryption operations (ms)"
    );
    describe_counter!(DECRYPTION_COUNT, "Number of decryption actions.");
    describe_counter!(DECRYPTION_ERROR_COUNT, "Number of decryption errors.");
    describe_histogram!(
        DECRYPTION_DURATION,
        Unit::Milliseconds,
        "Duration of decryption operations (ms)"
    );

    describe_counter!(STATEMENT_TOTAL_COUNT, "Total number of SQL statements .");
    describe_counter!(
        STATEMENT_ENCRYPTED_COUNT,
        "Number of encrypted SQL statements ."
    );
    describe_counter!(
        STATEMENT_PASSTHROUGH_COUNT,
        "Number of passthrough (non-encrypted) SQL statements ."
    );
    describe_histogram!(
        STATEMENT_DURATION,
        Unit::Milliseconds,
        "Duration of statement execution (ms)"
    );

    describe_gauge!(
        CLIENT_CONNECTION_COUNT,
        "Current number of client connections"
    );
    describe_counter!(CLIENT_BYTES_SENT, "Number of bytes sent to the client.");
    describe_counter!(
        CLIENT_BYTES_RECEIVED,
        "Number of bytes received from the client."
    );

    describe_counter!(SERVER_BYTES_SENT, "Number of bytes sent to the server.");
    describe_counter!(
        SERVER_BYTES_RECEIVED,
        "Number of bytes received from the server."
    );

    // Prometheus endpoint is empty on startup and looks like an error
    // Explicitly set count to zero
    gauge!(CLIENT_CONNECTION_COUNT).set(0);

    info!(msg = "Prometheus exporter started", port);
    Ok(())
}
