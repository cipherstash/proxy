use crate::error::{Error, ErrorChain};
use crate::prometheus::CLIENTS_ACTIVE_CONNECTIONS;
use metrics::gauge;
use std::future::Future;
use tracing::{error, info, warn};

///
/// Runs a client connection handler and tracks it in the
/// `CLIENTS_ACTIVE_CONNECTIONS` gauge.
///
/// Errors returned by the handler are logged at a level that matches their cause.
///
pub async fn track_connection<F>(handler: F)
where
    F: Future<Output = Result<(), Error>>,
{
    gauge!(CLIENTS_ACTIVE_CONNECTIONS).increment(1);

    match handler.await {
        Ok(_) => (),
        Err(err) => {
            gauge!(CLIENTS_ACTIVE_CONNECTIONS).decrement(1);

            match err {
                Error::ConnectionClosed => {
                    info!(msg = "Database connection closed by client");
                }
                Error::CancelRequest => {
                    info!(msg = "Database connection closed after cancel request");
                }
                Error::ConnectionTimeout { .. } => {
                    warn!(msg = "Database connection timeout", error = err.to_string());
                }
                _ => {
                    error!(msg = "Database connection error", error = %ErrorChain(&err));
                }
            }
        }
    }
}
