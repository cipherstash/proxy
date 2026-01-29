use crate::error::Error;
use crate::log::DEVELOPMENT;
use metrics::{describe_counter, describe_gauge, describe_histogram, gauge, Unit};
use metrics_exporter_prometheus::PrometheusBuilder;
use std::net::SocketAddr;
use tracing::{debug, info};

// See https://prometheus.io/docs/practices/naming/
pub const ENCRYPTED_VALUES_TOTAL: &str = "cipherstash_proxy_encrypted_values_total";
pub const ENCRYPTION_ERROR_TOTAL: &str = "cipherstash_proxy_encryption_error_total";
pub const ENCRYPTION_REQUESTS_TOTAL: &str = "cipherstash_proxy_encryption_requests_total";
pub const ENCRYPTION_DURATION_SECONDS: &str = "cipherstash_proxy_encryption_duration_seconds";

pub const DECRYPTED_VALUES_TOTAL: &str = "cipherstash_proxy_decrypted_values_total";
pub const DECRYPTION_ERROR_TOTAL: &str = "cipherstash_proxy_decryption_error_total";
pub const DECRYPTION_REQUESTS_TOTAL: &str = "cipherstash_proxy_decryption_requests_total";
pub const DECRYPTION_DURATION_SECONDS: &str = "cipherstash_proxy_decryption_duration_seconds";

pub const STATEMENTS_TOTAL: &str = "cipherstash_proxy_statements_total";
pub const STATEMENTS_ENCRYPTED_TOTAL: &str = "cipherstash_proxy_statements_encrypted_total";
pub const STATEMENTS_PASSTHROUGH_MAPPING_DISABLED_TOTAL: &str =
    "cipherstash_proxy_statements_passthrough_mapping_disabled_total";
pub const STATEMENTS_PASSTHROUGH_TOTAL: &str = "cipherstash_proxy_statements_passthrough_total";
pub const STATEMENTS_UNMAPPABLE_TOTAL: &str = "cipherstash_proxy_statements_unmappable_total";
pub const STATEMENTS_SESSION_DURATION_SECONDS: &str =
    "cipherstash_proxy_statements_session_duration_seconds";
pub const STATEMENTS_EXECUTION_DURATION_SECONDS: &str =
    "cipherstash_proxy_statements_execution_duration_seconds";
pub const SLOW_STATEMENTS_TOTAL: &str = "cipherstash_proxy_slow_statements_total";

pub const ROWS_TOTAL: &str = "cipherstash_proxy_rows_total";
pub const ROWS_ENCRYPTED_TOTAL: &str = "cipherstash_proxy_rows_encrypted_total";
pub const ROWS_PASSTHROUGH_TOTAL: &str = "cipherstash_proxy_rows_passthrough_total";

pub const CLIENTS_ACTIVE_CONNECTIONS: &str = "cipherstash_proxy_clients_active_connections";
pub const CLIENTS_BYTES_SENT_TOTAL: &str = "cipherstash_proxy_clients_bytes_sent_total";
pub const CLIENTS_BYTES_RECEIVED_TOTAL: &str = "cipherstash_proxy_clients_bytes_received_total";
pub const SERVER_BYTES_SENT_TOTAL: &str = "cipherstash_proxy_server_bytes_sent_total";
pub const SERVER_BYTES_RECEIVED_TOTAL: &str = "cipherstash_proxy_server_bytes_received_total";

pub const KEYSET_CIPHER_INIT_TOTAL: &str = "cipherstash_proxy_keyset_cipher_init_total";
pub const KEYSET_CIPHER_CACHE_HITS_TOTAL: &str = "cipherstash_proxy_keyset_cipher_cache_hits_total";
pub const KEYSET_CIPHER_INIT_DURATION_SECONDS: &str =
    "cipherstash_proxy_keyset_cipher_init_duration_seconds";

pub fn start(host: String, port: u16) -> Result<(), Error> {
    let address = format!("{host}:{port}");
    let socket_address: SocketAddr = address.parse().unwrap();

    debug!(target: DEVELOPMENT, msg = "Starting Prometheus exporter", port);

    PrometheusBuilder::new()
        .with_http_listener(socket_address)
        .install()?;

    describe_counter!(
        ENCRYPTED_VALUES_TOTAL,
        "Number of individual values that have been encrypted"
    );
    describe_counter!(
        ENCRYPTION_REQUESTS_TOTAL,
        "Number of requests to CipherStash ZeroKMS to encrypt values"
    );
    describe_counter!(
        ENCRYPTION_ERROR_TOTAL,
        "Number of encryption operations that were unsuccessful"
    );
    describe_histogram!(
        ENCRYPTION_DURATION_SECONDS,
        Unit::Seconds,
        "Duration of time CipherStash Proxy spent performing encryption operations"
    );
    describe_counter!(
        DECRYPTED_VALUES_TOTAL,
        "Number of individual values that have been decrypted"
    );
    describe_counter!(
        DECRYPTION_REQUESTS_TOTAL,
        "Number of requests to CipherStash ZeroKMS to decrypt values"
    );
    describe_counter!(
        DECRYPTION_ERROR_TOTAL,
        "Number of decryption operations that were unsuccessful"
    );
    describe_histogram!(
        DECRYPTION_DURATION_SECONDS,
        Unit::Seconds,
        "Duration of time CipherStash Proxy spent performing decryption operations"
    );

    describe_counter!(
        STATEMENTS_TOTAL,
        "Total number of SQL statements processed by CipherStash Proxy"
    );
    describe_counter!(
        STATEMENTS_ENCRYPTED_TOTAL,
        "Number of SQL statements that required encryption"
    );
    describe_counter!(
        STATEMENTS_PASSTHROUGH_MAPPING_DISABLED_TOTAL,
        "Number of SQL statements passed through while mapping was disabled"
    );
    describe_counter!(
        STATEMENTS_PASSTHROUGH_TOTAL,
        "Number of SQL statements that did not require encryption"
    );
    describe_counter!(
        STATEMENTS_UNMAPPABLE_TOTAL,
        "Total number of unmappable SQL statements processed by CipherStash Proxy"
    );
    describe_histogram!(
        STATEMENTS_SESSION_DURATION_SECONDS,
        Unit::Seconds,
        "Duration of time CipherStash Proxy spent processing the statement including encryption, proxied database execution, and decryption"
    );
    describe_histogram!(
        STATEMENTS_EXECUTION_DURATION_SECONDS,
        Unit::Seconds,
        "Duration of time the proxied database spent executing SQL statements"
    );
    describe_counter!(
        SLOW_STATEMENTS_TOTAL,
        "Total number of statements exceeding slow statement threshold"
    );

    describe_counter!(ROWS_TOTAL, "Total number of rows returned to clients");
    describe_counter!(
        ROWS_ENCRYPTED_TOTAL,
        "Number of encrypted rows returned to clients"
    );
    describe_counter!(
        ROWS_PASSTHROUGH_TOTAL,
        "Number of non-encrypted rows returned to clients"
    );

    describe_gauge!(
        CLIENTS_ACTIVE_CONNECTIONS,
        "Current number of connections to CipherStash Proxy from clients"
    );
    describe_counter!(
        CLIENTS_BYTES_SENT_TOTAL,
        "Number of bytes sent from CipherStash Proxy to clients"
    );
    describe_counter!(
        CLIENTS_BYTES_RECEIVED_TOTAL,
        "Number of bytes received by CipherStash Proxy from clients"
    );

    describe_counter!(
        SERVER_BYTES_SENT_TOTAL,
        "Number of bytes CipherStash Proxy sent to the PostgreSQL server"
    );
    describe_counter!(
        SERVER_BYTES_RECEIVED_TOTAL,
        "Number of bytes CipherStash Proxy received from the PostgreSQL server"
    );

    describe_counter!(
        KEYSET_CIPHER_INIT_TOTAL,
        "Number of times a new keyset-scoped cipher has been initialized"
    );
    describe_counter!(
        KEYSET_CIPHER_CACHE_HITS_TOTAL,
        "Number of times a keyset-scoped cipher was found in the cache"
    );
    describe_histogram!(
        KEYSET_CIPHER_INIT_DURATION_SECONDS,
        Unit::Seconds,
        "Duration of keyset-scoped cipher initialization (includes ZeroKMS network call)"
    );

    // Prometheus endpoint is empty on startup and looks like an error
    // Explicitly set count to zero
    gauge!(CLIENTS_ACTIVE_CONNECTIONS).set(0);

    info!(msg = "Prometheus exporter started", port);
    Ok(())
}
