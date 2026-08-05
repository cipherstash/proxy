use pg_proto::{
    codec::Backend, net::NetworkStream, pre_startup::Negotiation, transport::Buffered, Conn,
};
use std::sync::Arc;
use tokio::net::TcpStream;
use tracing::warn;

use crate::{
    error::{Error, ProtocolError},
    tls, TandemConfig,
};

/// Applies CipherStash's upstream TLS policy while pg-proto owns the wire
/// negotiation and its legal pre-startup transitions.
pub async fn with_tls(
    stream: NetworkStream<TcpStream>,
    config: &TandemConfig,
) -> Result<NetworkStream<TcpStream>, Error> {
    if config.database_tls_disabled() {
        warn!(msg = "Connecting to database without Transport Layer Security (TLS)");
        return Ok(stream);
    }

    let mut request = Conn::new(Buffered::<_, Backend>::new(stream)).request_ssl();
    request.flush().await?;
    match request.receive_ssl_reply().await? {
        Negotiation::Accepted(conn) => {
            let stream = conn
                .into_transport()
                .into_inner()
                .into_plain()
                .map_err(|_| ProtocolError::UnexpectedStartupMessage)?;
            let tls = pg_proto::tls::connect(
                stream,
                config.database.server_name()?.to_owned(),
                Arc::new(tls::configure_client(&config.database)),
            )
            .await?;
            Ok(NetworkStream::client_tls(tls))
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
