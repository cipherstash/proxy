use crate::{
    error::Error,
    postgresql::{SSL_RESPONSE_NO, SSL_RESPONSE_YES},
};

pub async fn connect_server(encrypt: &Encrypt) -> Result<TcpStream, Error> {
    let mut server_stream = TcpStream::connect(&encrypt.config.connect.to_socket_address()).await?;
    tcp::configure(&mut server_stream);

    if !encrypt.config.server.use_tls {
        return Ok(server_stream);
    }

    let server_ssl = send_ssl_request(&mut server_stream).await?;

    if !server_ssl {
        error!("Database cannot connect over TLS");
        return Err(ProtocolError::UnexpectedSSLResponse.into());
    }

    let mut root_cert_store = rustls::RootCertStore::empty();
    root_cert_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(root_cert_store)
        .with_no_client_auth();

    let mut dangerous = config.dangerous();
    dangerous.set_certificate_verifier(Arc::new(NoCertificateVerification {}));

    info!("Connecting to database over TLS");

    let connector = TlsConnector::from(Arc::new(config));

    let domain = encrypt.config.server.server_name()?.to_owned();
    let mut tls_stream = connector.connect(domain, server_stream).await?;

    let username = encrypt.config.connect.username.as_str();
    let database = encrypt.config.connect.database.as_str();

    info!("send_startup {username}/{database}");
    // send_startup(&mut tls_stream, username, database).await?;

    Ok(tls_stream.into_inner().0)
}

///
/// Send SSLRequest to the stream and return the response
/// Returns true if the server indicates support for TLS
///
async fn send_ssl_request<T: AsyncWrite + Unpin>(stream: &mut T) -> Result<bool, Error> {
    debug!("Send SSLRequest");
    let mut bytes = BytesMut::with_capacity(12);
    bytes.put_i32(8);
    bytes.put_i32(SSL_REQUEST);

    // Server supports TLS
    match server_stream.read_u8().await? {
        SSL_RESPONSE_YES => Ok(true),
        SSL_RESPONSE_NO => Ok(false),
        code => {
            error!("Unexpected startup message: {}", code as char);
            return Err(ProtocolError::UnexpectedStartupMessage.into());
        }
    }
}
