use super::backend::Backend;
use super::frontend::Frontend;
use super::protocol::StartupCode;
use crate::error::ConfigError;
use crate::log::{AUTHENTICATION, PROTOCOL};
use crate::postgresql::context::Context;
use crate::postgresql::messages::authentication::auth::{AuthenticationMethod, SaslMechanism};
use crate::postgresql::messages::authentication::sasl::SASLResponse;
use crate::postgresql::messages::authentication::{
    Authentication, PasswordMessage, SASLInitialResponse,
};
use crate::postgresql::messages::error_response::ErrorResponse;
use crate::postgresql::{protocol, startup};
use crate::{
    connect::AsyncStream,
    encrypt::Encrypt,
    error::{Error, ProtocolError},
    tls,
};
use bytes::BytesMut;
use md5::{Digest, Md5};
use postgres_protocol::authentication::sasl::{ChannelBinding, ScramSha256};
use rand::Rng;
use tokio::io::{split, AsyncRead, AsyncWrite, AsyncWriteExt};
use tracing::{debug, error, info, warn};

///
///
/// Entry point for handling postgres protocol connections
/// Each inbound client connection is mapped to a database connection
/// Hilarity ensues
///
/// Startup flow
///
///     Connect to database with TLS if required
///     First message is either:
///         - SSLRequest
///         - ProtocolVersionNumber
///         - CancelRequest
///
///     On SSLRequest
///         Send SSLResponse
///         Connect with TLS if configured
///
///         On TLS Connect
///             Expect message containing ProtocolVersionNumber is sent
///
///     On CancelRequest
///         Propagate and disconnect
///
///     On ProtocolVersionNumber
///         Propagate and continue
///
///
pub async fn handler(
    client_stream: AsyncStream,
    encrypt: Encrypt,
    client_id: i32,
) -> Result<(), Error> {
    let mut client_stream = client_stream;

    // Connect to the database server, using TLS if configured
    let stream = AsyncStream::connect(&encrypt.config.database.to_socket_address()).await?;
    let mut database_stream = startup::with_tls(stream, &encrypt.config).await?;
    info!(
        msg = "Client connected",
        database = encrypt.config.database.to_socket_address(),
        client_id = client_id,
    );
    loop {
        let startup_message = startup::read_message_with_timeout(
            &mut client_stream,
            encrypt.config.database.connection_timeout(),
        )
        .await?;

        match &startup_message.code {
            StartupCode::SSLRequest => {
                startup::send_ssl_response(&encrypt, &mut client_stream).await?;
                if let Some(ref tls) = encrypt.config.tls {
                    match client_stream {
                        AsyncStream::Tcp(stream) => {
                            // The Client is connecting to our Server
                            let tls_stream = tls::server(stream, tls).await?;
                            client_stream = AsyncStream::Tls(tls_stream);
                        }
                        AsyncStream::Tls(_) => {
                            unreachable!();
                        }
                    }
                }
            }
            StartupCode::CancelRequest => {
                database_stream.write_all(&startup_message.bytes).await?;
                return Err(Error::CancelRequest);
            }
            StartupCode::ProtocolVersionNumber => {
                database_stream.write_all(&startup_message.bytes).await?;
                break;
            }
        }
    }

    // Proxy -> Client Authentication
    // Uses MD5
    // SASL is not supported because I need to RTFM https://datatracker.ietf.org/doc/html/rfc5802
    //
    // Proxy -> Send AuthenticationMD5Password
    // Client -> Send PasswordMessage
    //
    {
        let salt = generate_md5_password_salt();

        let username = encrypt.config.database.username.as_bytes();
        let password = encrypt.config.database.password.as_bytes();

        let hash = md5_hash(username, password, &salt);

        let message = Authentication::md5_password(salt);
        let bytes = BytesMut::try_from(message)?;
        client_stream.write_all(&bytes).await?;

        let connection_timeout = encrypt.config.database.connection_timeout();
        let (_code, bytes) =
            protocol::read_message_with_timeout(&mut client_stream, client_id, connection_timeout)
                .await?;

        let password_message = PasswordMessage::try_from(&bytes)?;

        if hash == password_message.password {
            let message = Authentication::authentication_ok();
            debug!(target: AUTHENTICATION, msg = "Client AuthenticationOk");
            let bytes = BytesMut::try_from(message)?;
            client_stream.write_all(&bytes).await?;
        } else {
            let message = ErrorResponse::invalid_password(&encrypt.config.database.username);
            debug!(
                target: AUTHENTICATION,
                msg = "Client authenticaion failed: invalid password"
            );
            let bytes = BytesMut::try_from(message)?;
            client_stream.write_all(&bytes).await?;
            // return Err(ConfigError::TlsRequired.into());
        }
    }

    // Database authentication flow
    //   1. Database -> Authentication message (SASL)
    //               -> Proxy -> Auth Reponse flow with SASL
    //
    //   2. Proxy -> Auth message to the client Md5, SASL etc
    //            -> Client -> Auth response
    //

    // First message should always be Auth

    let auth = protocol::read_auth_message(&mut database_stream, client_id).await?;

    match &auth.method {
        AuthenticationMethod::AuthenticationOk => {
            debug!(target: AUTHENTICATION, msg = "AuthenticationOk");
        }
        AuthenticationMethod::AuthenticationCleartextPassword => {
            debug!(target: AUTHENTICATION, msg = "AuthenticationCleartextPassword");
            let password = encrypt.config.database.password.to_owned();
            let message = PasswordMessage::new(password);
            let bytes = BytesMut::try_from(message)?;
            database_stream.write_all(&bytes).await?;
        }
        AuthenticationMethod::Md5Password { salt } => {
            debug!(target: AUTHENTICATION, msg = "Md5Password");
            let username = encrypt.config.database.username.as_bytes();
            let password = encrypt.config.database.password.as_bytes();

            let hash = md5_hash(username, password, salt);
            let message = PasswordMessage::new(hash);
            let bytes = BytesMut::try_from(message)?;
            database_stream.write_all(&bytes).await?;
        }
        AuthenticationMethod::Sasl { .. } => {
            debug!(target: AUTHENTICATION, msg = "Sasl");
            let mechanism = auth.sasl_mechanism()?;

            sanity_check_sasl_mechanism(&mechanism, &client_stream);

            // Toby: I don't think we need to do anything here
            // If we are connected via TLS, we can support SCRAM-SHA-256-PLUS
            // If we are not connected via TLS, the database won't ask for SCRAM-SHA-256-PLUS
            let channel_binding = database_stream.channel_binding();
            let password = encrypt.config.database.password.as_bytes();
            scram_sha_256_plus_handler(&mut database_stream, mechanism, password, channel_binding)
                .await?;
        }
        AuthenticationMethod::Other { method_code, .. } => {
            debug!(target: AUTHENTICATION, msg = "UnsupportedAuthentication");
            return Err(ProtocolError::UnsupportedAuthentication {
                method_code: *method_code,
            }
            .into());
        }
        method => {
            debug!(target: AUTHENTICATION, msg = "UnexpectedStartupMessage", authentication_method = ?method);
            return Err(ProtocolError::UnexpectedStartupMessage.into());
        }
    }

    if encrypt.config.server.require_tls && !client_stream.is_tls() {
        let message = ErrorResponse::tls_required();
        let bytes = BytesMut::try_from(message)?;
        client_stream.write_all(&bytes).await?;

        error!(msg = "Client must connect with Transport Layer Security (TLS)");
        return Err(ConfigError::TlsRequired.into());
    }

    let (client_reader, client_writer) = split(client_stream);
    let (server_reader, server_writer) = split(database_stream);

    let schema = encrypt.schema.load();

    let context = Context::new(client_id, schema);

    let mut frontend = Frontend::new(
        client_reader,
        server_writer,
        encrypt.clone(),
        context.clone(),
    );
    let mut backend = Backend::new(
        client_writer,
        server_reader,
        encrypt.clone(),
        context.clone(),
    );

    if encrypt.is_passthrough() {
        warn!(msg = "⚠️ RUNNING IN PASSTHROUGH MODE");
        warn!(msg = "⛔️ DATA IS NOT PROTECTED WITH ENCRYPTION");
    }

    let client_to_server = async {
        loop {
            frontend.rewrite().await?;
        }
        // Unreachable, but helps the compiler understand the return type
        // TODO: extract into a function or something with type
        #[allow(unreachable_code)]
        Ok::<(), Error>(())
    };

    let server_to_client = async {
        loop {
            backend.rewrite().await?;
        }
        #[allow(unreachable_code)]
        Ok::<(), Error>(())
    };

    tokio::try_join!(client_to_server, server_to_client)?;

    Ok(())
}

// Keep for debugging
fn sanity_check_sasl_mechanism(mechanism: &SaslMechanism, client_stream: &AsyncStream) {
    match mechanism {
        SaslMechanism::ScramSha256 => {
            if client_stream.is_tls() {
                debug!(
                    PROTOCOL,
                    msg = "Database requested SCRAM-SHA-256, but Proxy has a TLS connection"
                );
            }
        }
        SaslMechanism::ScramSha256Plus => {
            if client_stream.is_tcp() {
                debug!(
                    PROTOCOL,
                    msg = "Database requested SCRAM-SHA-256-PLUS, but Proxy has a TCP connection"
                );
            }
        }
    }
}

pub fn md5_hash(username: &[u8], password: &[u8], salt: &[u8; 4]) -> String {
    let mut md5 = Md5::new();
    md5.update(password);
    md5.update(username);
    let output = md5.finalize_reset();
    md5.update(format!("{:x}", output));
    md5.update(salt);
    format!("md5{:x}", md5.finalize())
}

fn generate_md5_password_salt() -> [u8; 4] {
    let mut rng = rand::rng();
    let mut bytes = [0u8; 4];
    rng.fill(&mut bytes);
    bytes
}

async fn scram_sha_256_plus_handler<S: AsyncRead + AsyncWrite + Unpin>(
    mut stream: S,
    mechanism: SaslMechanism,
    password: &[u8],
    channel_binding: ChannelBinding,
) -> Result<(), Error> {
    let mut scram = ScramSha256::new(password, channel_binding);
    let bytes = scram.message().to_vec();

    let sasl_initial_response = SASLInitialResponse::new(mechanism, bytes);
    let bytes = BytesMut::try_from(sasl_initial_response)?;
    stream.write_all(&bytes).await?;

    let auth = protocol::read_auth_message(&mut stream, 1).await?;

    let bytes = auth.sasl_continue()?;
    scram.update(bytes)?;

    let sasl_response = SASLResponse::new(scram.message().to_vec());

    let bytes = BytesMut::try_from(sasl_response)?;
    stream.write_all(&bytes).await?;

    let auth = protocol::read_auth_message(&mut stream, 1).await?;
    let bytes = auth.sasl_final()?;
    scram.finish(bytes)?;

    let auth = protocol::read_auth_message(&mut stream, 1).await?;

    if auth.is_ok() {
        debug!(target: AUTHENTICATION, msg = "SASL authentication successful");
        Ok(())
    } else {
        Err(ProtocolError::AuthenticationFailed.into())
    }
}
