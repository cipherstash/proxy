use super::backend::Backend;
use super::frontend::Frontend;
use crate::connect::ChannelWriter;
use crate::error::ConfigError;
use crate::log::AUTHENTICATION;
use crate::postgresql::messages::error_response::ErrorResponse;
use crate::postgresql::{protocol, startup};
use crate::proxy::ZeroKms;
use crate::{
    connect::AsyncStream,
    error::{Error, ProtocolError},
    postgresql::context::Context,
    tls,
};
use bytes::{BufMut, Bytes, BytesMut};
use md5::{Digest, Md5};
use pg_proto::pre_startup::PreStartupMessage;
use pg_proto::{
    codec::{
        Authentication, Backend as BackendDirection, BackendMessage, Frontend as FrontendDirection,
        FrontendMessage,
    },
    pre_startup::{PreStartup, PreStartupOffer},
    server_auth::{ServerPassword, ServerProtocolOffer},
    startup::ProtocolVersion,
    transport::Buffered,
    Conn,
};
use postgres_protocol::authentication::sasl::{ChannelBinding, ScramSha256};
use rand::Rng;
use std::time::Duration;
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    time::timeout,
};
use tracing::{debug, error, info, warn};

const SCRAM_SHA_256_PLUS: &[u8] = b"SCRAM-SHA-256-PLUS";
const SCRAM_SHA_256: &[u8] = b"SCRAM-SHA-256";

async fn receive_pre_startup<S: AsyncRead + Unpin>(
    mut conn: Conn<Buffered<S, FrontendDirection>, PreStartup>,
    connection_timeout: Option<Duration>,
) -> Result<
    (
        Conn<Buffered<S, FrontendDirection>, PreStartup>,
        PreStartupMessage,
    ),
    (Conn<Buffered<S, FrontendDirection>, PreStartup>, Error),
> {
    let received = match connection_timeout {
        Some(duration) => match timeout(duration, conn.receive_pre_startup_wire()).await {
            Ok(received) => received,
            Err(_) => return Err((conn, Error::ConnectionTimeout { duration })),
        },
        None => conn.receive_pre_startup_wire().await,
    };
    match received {
        Ok(message) => Ok((conn, message)),
        Err(error) => Err((conn, error.into())),
    }
}

async fn receive_frontend_auth<S: AsyncRead + Unpin>(
    mut conn: Conn<Buffered<S, FrontendDirection>, ServerPassword>,
    connection_timeout: Option<Duration>,
) -> Result<
    (
        Conn<Buffered<S, FrontendDirection>, ServerPassword>,
        FrontendMessage,
    ),
    (Conn<Buffered<S, FrontendDirection>, ServerPassword>, Error),
> {
    let received = match connection_timeout {
        Some(duration) => match timeout(duration, conn.receive_frontend_wire()).await {
            Ok(received) => received,
            Err(_) => return Err((conn, Error::ConnectionTimeout { duration })),
        },
        None => conn.receive_frontend_wire().await,
    };
    match received {
        Ok(message) => Ok((conn, message)),
        Err(error) => Err((conn, error.into())),
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum SaslMechanism {
    ScramSha256,
    ScramSha256Plus,
}
/// Handles one downstream PostgreSQL connection and its paired upstream connection.
///
/// Negotiation and message validation are delegated to `pg-proto`; this function
/// retains the proxy-specific TLS policy, authentication policy, and forwarding.
pub async fn handler(client_stream: AsyncStream, context: Context<ZeroKms>) -> Result<(), Error> {
    let mut client_is_tls = client_stream.is_tls();
    let mut client = Conn::new(Buffered::<_, FrontendDirection>::new_frontend(
        client_stream,
    ));
    let client_id = context.client_id;

    // Connect to the database server, using TLS if configured
    let stream = AsyncStream::connect(&context.database_socket_address()).await?;
    let mut database_stream = startup::with_tls(stream, context.config()).await?;
    let database_channel_binding = database_stream.channel_binding();
    info!(
        msg = "Client connected",
        database = context.database_socket_address(),
        client_id = client_id,
    );

    let (client_startup, startup_message) = loop {
        let (pre_startup, startup_message) =
            match receive_pre_startup(client, context.connection_timeout()).await {
                Ok(result) => result,
                Err((conn, err @ Error::ConnectionTimeout { .. })) => {
                    let mut transport = conn.into_transport();
                    send_timeout_error(&mut transport, &err).await;
                    return Err(err);
                }
                Err((conn, err)) => {
                    conn.into_transport();
                    return Err(err);
                }
            };

        match pre_startup.offer_pre_startup(startup_message) {
            PreStartupOffer::Ssl(decision) => {
                let mut stream = decision.into_transport().into_inner();
                startup::send_ssl_response(&mut stream, context.use_tls()).await?;
                if let Some(ref tls) = context.tls_config() {
                    stream = match stream {
                        AsyncStream::Tcp(tcp_stream) => {
                            // The Client is connecting to our Server
                            let tls_stream = tls::server(tcp_stream, tls).await?;
                            client_is_tls = true;
                            AsyncStream::Tls(Box::new(tls_stream))
                        }
                        AsyncStream::Tls(_) => {
                            unreachable!();
                        }
                    };
                }
                client = Conn::new(Buffered::new_frontend(stream));
            }
            PreStartupOffer::Cancel {
                conn,
                process_id,
                secret_key,
            } => {
                conn.into_transport();
                let startup_message = PreStartupMessage::CancelRequest {
                    process_id,
                    secret_key,
                };
                database_stream
                    .write_all(&startup_message.to_packet()?)
                    .await?;
                return Err(Error::CancelRequest);
            }
            PreStartupOffer::Startup { conn, message } => {
                let startup_message = PreStartupMessage::Startup(message.clone());
                database_stream
                    .write_all(&startup_message.to_packet()?)
                    .await?;
                break (conn, message);
            }
            PreStartupOffer::Gss(conn) => {
                conn.into_transport();
                return Err(ProtocolError::UnexpectedStartupMessage.into());
            }
        }
    };

    let mut database_stream = Buffered::<_, BackendDirection>::new(database_stream);

    // Proxy -> Client Authentication
    // Uses MD5
    // SASL is not supported because I need to RTFM https://datatracker.ietf.org/doc/html/rfc5802
    //
    // Proxy -> Send AuthenticationMD5Password
    // Client -> Send PasswordMessage
    //
    let client_validated =
        match client_startup.validate_protocol(startup_message, ProtocolVersion::V3_2) {
            ServerProtocolOffer::Supported { conn, .. } => conn,
            ServerProtocolOffer::Rejected { conn, .. } => {
                conn.into_transport();
                return Err(ProtocolError::UnexpectedStartupMessage.into());
            }
        };

    let mut client_stream = {
        let salt = generate_md5_password_salt();

        let username = context.database_username().as_bytes();
        let password = context.database_password();

        let password = password.as_bytes();

        let hash = md5_hash(username, password, &salt);

        let (mut password_state, frame) = client_validated.begin_server_auth().request_md5(salt)?;
        password_state.push_frame(frame)?;
        password_state.flush().await?;

        let connection_timeout = context.connection_timeout();
        let (password_state, message) =
            match receive_frontend_auth(password_state, connection_timeout).await {
                Ok(result) => result,
                Err((conn, err @ Error::ConnectionTimeout { .. })) => {
                    let mut transport = conn.into_transport();
                    send_timeout_error(&mut transport, &err).await;
                    return Err(err);
                }
                Err((conn, err)) => {
                    conn.into_transport();
                    return Err(err);
                }
            };

        let (auth_state, password) =
            password_state
                .receive_password(message)
                .map_err(|rejected| {
                    let (conn, _message) = *rejected;
                    conn.into_transport();
                    ProtocolError::UnexpectedAuthenticationResponse {
                        expected: "PasswordResponse".into(),
                        received: -1,
                    }
                })?;
        let password =
            std::str::from_utf8(&password).map_err(|_| ProtocolError::AuthenticationFailed)?;

        if hash != password {
            auth_state.into_transport();
            return Err(ProtocolError::ClientAuthenticationFailed.into());
        }

        debug!(target: AUTHENTICATION, msg = "Client AuthenticationOk");
        let (mut startup_ready, frame) = auth_state.authentication_ok()?;
        startup_ready.push_frame(frame)?;
        startup_ready.flush().await?;
        startup_ready.into_transport()
    };

    // Database authentication flow
    //   1. Database -> Authentication message (SASL)
    //               -> Proxy -> Auth Reponse flow with SASL
    //
    //   2. Proxy -> Auth message to the client Md5, SASL etc
    //            -> Client -> Auth response
    //

    // First message should always be Auth
    let auth = protocol::read_auth_message(&mut database_stream).await?;

    match &auth {
        Authentication::Ok => {
            debug!(target: AUTHENTICATION, msg = "AuthenticationOk");
        }
        Authentication::CleartextPassword => {
            debug!(target: AUTHENTICATION, msg = "AuthenticationCleartextPassword");
            let password = context.database_password();
            send_frontend_message(&mut database_stream, password_message(password)?).await?;
        }
        Authentication::Md5Password { salt } => {
            debug!(target: AUTHENTICATION, msg = "Md5Password");
            let username = context.database_username().as_bytes();
            let password = context.database_password();
            let password = password.as_bytes();

            let hash = md5_hash(username, password, salt);
            send_frontend_message(&mut database_stream, password_message(hash)?).await?;
        }
        Authentication::Sasl { mechanisms } => {
            debug!(target: AUTHENTICATION, msg = "Sasl");
            let mechanism = sasl_mechanism(mechanisms)?;
            let password = context.database_password();
            let password = password.as_bytes();
            scram_sha_256_plus_handler(
                &mut database_stream,
                mechanism,
                password,
                database_channel_binding,
            )
            .await?;
        }
        Authentication::KerberosV5
        | Authentication::Gss
        | Authentication::GssContinue(_)
        | Authentication::Sspi => {
            debug!(target: AUTHENTICATION, msg = "UnsupportedAuthentication");
            return Err(ProtocolError::UnsupportedAuthentication {
                method_code: authentication_method_code(&auth),
            }
            .into());
        }
        Authentication::SaslContinue(_) | Authentication::SaslFinal(_) => {
            debug!(target: AUTHENTICATION, msg = "UnexpectedStartupMessage", authentication_method = ?auth);
            return Err(ProtocolError::UnexpectedStartupMessage.into());
        }
    }

    if context.require_tls() && !client_is_tls {
        let message = ErrorResponse::tls_required();
        send_backend_message(&mut client_stream, message.into_backend_message()).await?;

        error!(msg = "Client must connect with Transport Layer Security (TLS)");
        return Err(ConfigError::TlsRequired.into());
    }

    let (client_reader, client_writer) = client_stream.into_inner().split();
    let (server_reader, server_writer) = database_stream.into_inner().split();

    let channel_writer = ChannelWriter::new(client_writer, client_id);

    let mut frontend = Frontend::new(
        client_reader,
        channel_writer.sender(),
        server_writer,
        context.clone(),
    );
    let mut backend = Backend::new(channel_writer.sender(), server_reader, context.clone());

    if context.is_passthrough() {
        if context.use_structured_logging() {
            warn!(msg = "RUNNING IN PASSTHROUGH MODE");
            warn!(msg = "DATA IS NOT PROTECTED WITH ENCRYPTION");
        } else {
            warn!(msg = "========================================");
            warn!(msg = "RUNNING IN PASSTHROUGH MODE");
            warn!(msg = "DATA IS NOT PROTECTED WITH ENCRYPTION");
            warn!(msg = "========================================");
        }
    }

    let timeout_sender = channel_writer.sender();
    let channel_writer_task = tokio::spawn(channel_writer.receive());

    let client_to_server = async {
        loop {
            let result = frontend.rewrite().await;
            // Ensure the connection is terminated if the client closes the connection
            // The client ConnectionClosed error is triggered before the terminate message is passed through
            if matches!(result, Err(Error::ConnectionClosed)) {
                frontend.terminate().await?
            }
            result?;
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

    // Run frontend and backend tasks
    let result = tokio::try_join!(client_to_server, server_to_client);

    if let Err(ref err @ Error::ConnectionTimeout { .. }) = &result {
        let error_response = ErrorResponse::connection_timeout(err.to_string());
        if let Ok(bytes) = protocol::encode_backend_message(&error_response.into_backend_message())
        {
            let _ = timeout_sender.send(bytes);
        }
        // Best-effort yield to allow ChannelWriter to flush the error response
        // before the connection tears down. Not guaranteed — if the runtime doesn't
        // schedule the writer task before teardown, the client may see a connection
        // reset instead of the ErrorResponse.
        tokio::task::yield_now().await;
    }

    // Drop frontend and backend to drop their senders and close the channel
    // The async blocks above captured frontend/backend by reference, so they're still alive
    drop(frontend);
    drop(backend);

    // Wait for channel writer to finish shutdown sequence
    // The senders are now dropped, which closes the channel and allows
    // the writer task to complete its shutdown
    if let Err(err) = channel_writer_task.await {
        error!(
            client_id,
            msg = "Channel writer task panicked",
            error = ?err
        );
    }

    result?;
    Ok(())
}

fn sasl_mechanism(mechanisms: &[Bytes]) -> Result<SaslMechanism, Error> {
    match mechanisms.first().map(Bytes::as_ref) {
        Some(SCRAM_SHA_256) => Ok(SaslMechanism::ScramSha256),
        Some(SCRAM_SHA_256_PLUS) => Ok(SaslMechanism::ScramSha256Plus),
        Some(mechanism) => Err(ProtocolError::UnexpectedSaslAuthenticationMethod(
            String::from_utf8_lossy(mechanism).into_owned(),
        )
        .into()),
        None => Err(ProtocolError::UnexpectedSaslAuthenticationMethod("None".to_string()).into()),
    }
}

fn authentication_method_code(authentication: &Authentication) -> i32 {
    match authentication {
        Authentication::Ok => 0,
        Authentication::KerberosV5 => 2,
        Authentication::CleartextPassword => 3,
        Authentication::Md5Password { .. } => 5,
        Authentication::Gss => 7,
        Authentication::GssContinue(_) => 8,
        Authentication::Sspi => 9,
        Authentication::Sasl { .. } => 10,
        Authentication::SaslContinue(_) => 11,
        Authentication::SaslFinal(_) => 12,
    }
}

fn password_message(password: String) -> Result<FrontendMessage, Error> {
    let password = std::ffi::CString::new(password)?;
    Ok(FrontendMessage::PasswordResponse(Bytes::copy_from_slice(
        password.as_bytes_with_nul(),
    )))
}

pub fn md5_hash(username: &[u8], password: &[u8], salt: &[u8; 4]) -> String {
    let mut md5 = Md5::new();
    md5.update(password);
    md5.update(username);
    let output = md5.finalize_reset();
    md5.update(format!("{output:x}"));
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
    stream: &mut Buffered<S, BackendDirection>,
    mechanism: SaslMechanism,
    password: &[u8],
    channel_binding: ChannelBinding,
) -> Result<(), Error> {
    let mut scram = ScramSha256::new(password, channel_binding);
    let bytes = scram.message().to_vec();

    let mechanism = match mechanism {
        SaslMechanism::ScramSha256 => SCRAM_SHA_256,
        SaslMechanism::ScramSha256Plus => SCRAM_SHA_256_PLUS,
    };
    let mut initial = BytesMut::new();
    initial.extend_from_slice(mechanism);
    initial.put_u8(0);
    initial.put_i32(bytes.len().try_into().map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "SASL response is too large",
        )
    })?);
    initial.extend_from_slice(&bytes);
    send_frontend_message(stream, FrontendMessage::PasswordResponse(initial.freeze())).await?;

    let auth = protocol::read_auth_message(stream).await?;

    let Authentication::SaslContinue(bytes) = auth else {
        return Err(ProtocolError::UnexpectedAuthenticationResponse {
            expected: "SaslContinue".into(),
            received: authentication_method_code(&auth),
        }
        .into());
    };
    scram.update(&bytes)?;

    send_frontend_message(
        stream,
        FrontendMessage::PasswordResponse(Bytes::copy_from_slice(scram.message())),
    )
    .await?;

    let auth = protocol::read_auth_message(stream).await?;
    let Authentication::SaslFinal(bytes) = auth else {
        return Err(ProtocolError::UnexpectedAuthenticationResponse {
            expected: "SaslFinal".into(),
            received: authentication_method_code(&auth),
        }
        .into());
    };
    scram.finish(&bytes)?;

    let auth = protocol::read_auth_message(stream).await?;

    if matches!(auth, Authentication::Ok) {
        debug!(target: AUTHENTICATION, msg = "SASL authentication successful");
        Ok(())
    } else {
        Err(ProtocolError::AuthenticationFailed.into())
    }
}

/// Best-effort send of a connection timeout ErrorResponse directly to a client stream.
/// Used for pre-split timeout sites where no ChannelWriter exists yet.
async fn send_timeout_error<S: AsyncWrite + Unpin>(
    stream: &mut Buffered<S, FrontendDirection>,
    err: &Error,
) {
    let error_response = ErrorResponse::connection_timeout(err.to_string());
    let _ = send_backend_message(stream, error_response.into_backend_message()).await;
}

async fn send_backend_message<S: AsyncWrite + Unpin>(
    stream: &mut Buffered<S, FrontendDirection>,
    message: BackendMessage,
) -> Result<(), Error> {
    stream.push(message.to_frame()?)?;
    stream.flush().await?;
    Ok(())
}

async fn send_frontend_message<S: AsyncWrite + Unpin>(
    stream: &mut Buffered<S, BackendDirection>,
    message: FrontendMessage,
) -> Result<(), Error> {
    stream.push(message.to_frame()?)?;
    stream.flush().await?;
    Ok(())
}
