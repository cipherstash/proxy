use pg_proto::{codec::Backend, pre_startup::Negotiation, transport::Buffered, Conn};
use tracing::warn;

use crate::{
    connect::AsyncStream,
    error::{Error, ProtocolError},
    tls, TandemConfig,
};

/// Applies CipherStash's upstream TLS policy while pg-proto owns the wire
/// negotiation and its legal pre-startup transitions.
pub async fn with_tls(stream: AsyncStream, config: &TandemConfig) -> Result<AsyncStream, Error> {
    if config.database_tls_disabled() {
        warn!(msg = "Connecting to database without Transport Layer Security (TLS)");
        return Ok(stream);
    }

    let mut request = Conn::new(Buffered::<_, Backend>::new(stream)).request_ssl();
    request.flush().await?;
    match request.receive_ssl_reply().await? {
        Negotiation::Accepted(conn) => {
            let AsyncStream::Tcp(stream) = conn.into_transport().into_inner() else {
                return Err(ProtocolError::UnexpectedStartupMessage.into());
            };
            Ok(AsyncStream::Tls(Box::new(
                tls::client(stream, config).await?,
            )))
        }
        Negotiation::Rejected(conn) => {
            warn!(msg = "Connecting to database without Transport Layer Security (TLS)");
            Ok(conn.into_transport().into_inner())
        }
        Negotiation::LegacyError(conn) => {
            conn.into_transport();
            Err(ProtocolError::UnexpectedStartupMessage.into())
        }
    }
}
