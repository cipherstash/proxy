use super::middleware::{Backend, BackendDisposition, Frontend, FrontendDisposition};
use crate::connect::{self, ChannelWriter};
use crate::error::ConfigError;
use crate::log::AUTHENTICATION;
use crate::postgresql::diagnostics::ErrorResponse;
use crate::prometheus::{
    CLIENTS_BYTES_RECEIVED_TOTAL, SERVER_BYTES_RECEIVED_TOTAL, SERVER_BYTES_SENT_TOTAL,
};
use crate::proxy::ZeroKms;
use crate::{
    error::{Error, ProtocolError},
    postgresql::context::Context,
    tls, TandemConfig,
};
use bytes::Bytes;
use md5::{Digest, Md5};
use metrics::counter;
use pg_proto::pre_startup::PreStartupMessage;
use pg_proto::{
    auth::{AuthCompletion, AuthEvent, AuthOffer, AwaitingStartupReady, SaslEvent},
    codec::{
        Backend as BackendDirection, BackendMessage, Frontend as FrontendDirection, FrontendMessage,
    },
    middleware::{Identity, Inbound, Middleware, PhaseAssociation, ServerRole, TypedReceiveError},
    net::NetworkStream,
    pre_startup::{Negotiation, PreStartup, PreStartupOffer},
    server_auth::{ServerPassword, ServerProtocolOffer},
    startup::ProtocolVersion,
    transport::Buffered,
    Conn,
};
use postgres_protocol::authentication::sasl::{ChannelBinding, ScramSha256};
use rand::Rng;
use std::{
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::{
    io::{AsyncRead, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    time::timeout,
};
use tracing::{debug, error, info, warn};

const SCRAM_SHA_256_PLUS: &[u8] = b"SCRAM-SHA-256-PLUS";
const SCRAM_SHA_256: &[u8] = b"SCRAM-SHA-256";
type PgStream = NetworkStream<TcpStream>;
type ProtocolMiddleware = Middleware<(), Identity>;

fn typed_receive_error<Message>(
    error: TypedReceiveError<std::convert::Infallible, Message>,
) -> Error
where
    Message: std::fmt::Debug,
{
    match error {
        TypedReceiveError::Io(error) => error.into(),
        TypedReceiveError::Illegal(message) => std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("message is illegal in the current PostgreSQL phase: {message:?}"),
        )
        .into(),
        TypedReceiveError::Middleware(never) => match never {},
        TypedReceiveError::InvalidWire(message) => std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            format!("middleware produced an invalid PostgreSQL message: {message:?}"),
        )
        .into(),
    }
}

async fn receive_pre_startup<S: AsyncRead + Unpin>(
    mut conn: Conn<Buffered<S, FrontendDirection>, PreStartup>,
    middleware: &mut ProtocolMiddleware,
    connection_timeout: Option<Duration>,
) -> Result<
    (
        Conn<Buffered<S, FrontendDirection>, PreStartup>,
        PreStartupMessage,
    ),
    (Conn<Buffered<S, FrontendDirection>, PreStartup>, Error),
> {
    let received = match connection_timeout {
        Some(duration) => match timeout(duration, conn.receive_pre_startup_typed(middleware)).await
        {
            Ok(received) => received,
            Err(_) => return Err((conn, Error::ConnectionTimeout { duration })),
        },
        None => conn.receive_pre_startup_typed(middleware).await,
    };
    match received {
        Ok(message) => Ok((conn, message.into_wire())),
        Err(error) => Err((conn, typed_receive_error(error))),
    }
}

async fn receive_frontend_auth<S: AsyncRead + Unpin>(
    mut conn: Conn<Buffered<S, FrontendDirection>, ServerPassword>,
    middleware: &mut ProtocolMiddleware,
    connection_timeout: Option<Duration>,
) -> Result<
    (
        Conn<Buffered<S, FrontendDirection>, ServerPassword>,
        FrontendMessage,
    ),
    (Conn<Buffered<S, FrontendDirection>, ServerPassword>, Error),
> {
    let received = match connection_timeout {
        Some(duration) => match timeout(duration, conn.receive_frontend_typed(middleware)).await {
            Ok(received) => received,
            Err(_) => return Err((conn, Error::ConnectionTimeout { duration })),
        },
        None => conn.receive_frontend_typed(middleware).await,
    };
    match received {
        Ok(message) => Ok((conn, message.into_wire())),
        Err(error) => Err((conn, typed_receive_error(error))),
    }
}

async fn receive_backend_conn<S, Phase>(
    mut conn: Conn<Buffered<S, BackendDirection>, Phase>,
    middleware: &mut ProtocolMiddleware,
) -> Result<(Conn<Buffered<S, BackendDirection>, Phase>, BackendMessage), Error>
where
    S: AsyncRead + Unpin,
    Phase: PhaseAssociation<Inbound, ServerRole, BackendMessage>,
    <Phase as PhaseAssociation<Inbound, ServerRole, BackendMessage>>::Message: Into<BackendMessage>,
{
    let duration = Duration::from_secs(10);
    let message = timeout(duration, conn.receive_backend_typed(middleware))
        .await
        .map_err(|_| Error::ConnectionTimeout { duration })?
        .map_err(typed_receive_error)?;
    Ok((conn, message.into()))
}

async fn authenticate_upstream(
    startup: Conn<Buffered<PgStream, BackendDirection>, pg_proto::pre_startup::Startup>,
    context: &Context<ZeroKms>,
    middleware: &mut ProtocolMiddleware,
) -> Result<Conn<Buffered<PgStream, BackendDirection>, AwaitingStartupReady>, Error> {
    let mut auth = startup.authentication();
    let offer = loop {
        let (current, message) = receive_backend_conn(auth, middleware).await?;
        match current.offer_backend(message) {
            Ok(AuthEvent::Authentication(offer)) => break offer,
            Ok(AuthEvent::Negotiate { conn, .. }) => auth = conn,
            Ok(AuthEvent::Error { conn, .. }) => {
                conn.into_transport();
                return Err(ProtocolError::AuthenticationFailed.into());
            }
            Err((conn, _message, source)) => {
                conn.into_transport();
                return Err(source
                    .unwrap_or_else(|| {
                        std::io::Error::new(
                            std::io::ErrorKind::InvalidData,
                            "unexpected message during upstream authentication",
                        )
                    })
                    .into());
            }
        }
    };

    match offer {
        AuthOffer::Ok(conn) => Ok(conn),
        AuthOffer::Cleartext(conn) => {
            let (mut awaiting, frame) = conn.password(context.database_password().as_bytes())?;
            awaiting.push_frame(frame)?;
            awaiting.flush().await?;
            complete_upstream_auth(awaiting, middleware).await
        }
        AuthOffer::Md5 { conn, salt } => {
            let hash = md5_hash(
                context.database_username().as_bytes(),
                context.database_password().as_bytes(),
                &salt,
            );
            let (mut awaiting, frame) = conn.password(hash.as_bytes())?;
            awaiting.push_frame(frame)?;
            awaiting.flush().await?;
            complete_upstream_auth(awaiting, middleware).await
        }
        AuthOffer::Sasl { conn, mechanisms } => {
            let mechanism = sasl_mechanism(&mechanisms)?;
            let mut scram = ScramSha256::new(
                context.database_password().as_bytes(),
                match mechanism {
                    SaslMechanism::ScramSha256 => ChannelBinding::unsupported(),
                    SaslMechanism::ScramSha256Plus => {
                        ChannelBinding::tls_server_end_point(conn.tls_server_end_point().to_vec())
                    }
                },
            );
            let (mut sasl, frame) = match mechanism {
                SaslMechanism::ScramSha256 => conn.scram_sha_256(scram.message())?,
                SaslMechanism::ScramSha256Plus => conn.scram_sha_256_plus(scram.message())?,
            };
            sasl.push_frame(frame)?;
            sasl.flush().await?;

            let (sasl, message) = receive_backend_conn(sasl, middleware).await?;
            let BackendMessage::Authentication(authentication) = message else {
                sasl.into_transport();
                return Err(ProtocolError::UnexpectedStartupMessage.into());
            };
            let SaslEvent::Continue {
                conn: challenge,
                challenge: server_first,
            } = sasl.offer(authentication).map_err(|(conn, _)| {
                conn.into_transport();
                ProtocolError::AuthenticationFailed
            })?
            else {
                return Err(ProtocolError::AuthenticationFailed.into());
            };
            scram.update(&server_first)?;
            let (mut sasl, frame) = challenge.respond(Bytes::copy_from_slice(scram.message()));
            sasl.push_frame(frame)?;
            sasl.flush().await?;

            let (sasl, message) = receive_backend_conn(sasl, middleware).await?;
            let BackendMessage::Authentication(authentication) = message else {
                sasl.into_transport();
                return Err(ProtocolError::UnexpectedStartupMessage.into());
            };
            let SaslEvent::Final {
                conn: final_state,
                server_final,
            } = sasl.offer(authentication).map_err(|(conn, _)| {
                conn.into_transport();
                ProtocolError::AuthenticationFailed
            })?
            else {
                return Err(ProtocolError::AuthenticationFailed.into());
            };
            scram.finish(&server_final)?;
            complete_upstream_auth(final_state.verified(), middleware).await
        }
        AuthOffer::Gss(conn) | AuthOffer::Sspi(conn) | AuthOffer::KerberosV5(conn) => {
            conn.into_transport();
            Err(ProtocolError::UnsupportedAuthentication { method_code: -1 }.into())
        }
    }
}

async fn complete_upstream_auth(
    awaiting: Conn<Buffered<PgStream, BackendDirection>, pg_proto::auth::AwaitingAuthOk>,
    middleware: &mut ProtocolMiddleware,
) -> Result<Conn<Buffered<PgStream, BackendDirection>, AwaitingStartupReady>, Error> {
    let (awaiting, message) = receive_backend_conn(awaiting, middleware).await?;
    match awaiting.offer(message) {
        Ok(AuthCompletion::Ok(conn)) => Ok(conn),
        Ok(AuthCompletion::Error { conn, .. }) => {
            conn.into_transport();
            Err(ProtocolError::AuthenticationFailed.into())
        }
        Err((conn, _)) => {
            conn.into_transport();
            Err(ProtocolError::AuthenticationFailed.into())
        }
    }
}

async fn drain_upstream_startup(
    mut database: Conn<Buffered<PgStream, BackendDirection>, AwaitingStartupReady>,
    client: &mut Buffered<PgStream, FrontendDirection>,
    middleware: &mut ProtocolMiddleware,
) -> Result<Buffered<PgStream, BackendDirection>, Error> {
    loop {
        let typed = database
            .receive_backend_typed(middleware)
            .await
            .map_err(typed_receive_error)?;
        let message: BackendMessage = typed.into();
        let session_item = database.project_backend(message.clone());
        send_backend_message(client, message).await?;
        if let Some(item) = session_item {
            match database.offer_ready(item) {
                Ok(ready) => return Ok(ready.into_transport()),
                Err((conn, _)) => database = conn,
            }
        }
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
pub async fn handler(client_stream: PgStream, context: Context<ZeroKms>) -> Result<(), Error> {
    let mut downstream_middleware = Middleware::new((), Identity);
    let mut upstream_middleware = Middleware::new((), Identity);
    let mut client_is_tls = client_stream.is_tls();
    let mut client = Conn::new(Buffered::<_, FrontendDirection>::new_frontend(
        client_stream,
    ));
    let client_id = context.client_id;

    // Connect to the database server, using TLS if configured
    let stream = connect::connect(&context.database_socket_address()).await?;
    let mut database_stream = connect_upstream_tls(stream, context.config()).await?;
    info!(
        msg = "Client connected",
        database = context.database_socket_address(),
        client_id = client_id,
    );

    let (client_startup, startup_message) = loop {
        let (pre_startup, startup_message) = match receive_pre_startup(
            client,
            &mut downstream_middleware,
            context.connection_timeout(),
        )
        .await
        {
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
                let mut stream = if context.use_tls() {
                    let (accepted, reply) = decision.accept_ssl();
                    let mut stream = accepted.into_transport().into_inner();
                    stream.write_all(&[reply]).await?;
                    stream
                } else {
                    let (rejected, reply) = decision.reject_ssl();
                    let mut stream = rejected.into_transport().into_inner();
                    stream.write_all(&[reply]).await?;
                    stream
                };
                if let Some(ref tls) = context.tls_config() {
                    let tcp_stream = stream
                        .into_plain()
                        .map_err(|_| ProtocolError::UnexpectedStartupMessage)?;
                    let (server_config, leaf) = tls::configure_server_with_leaf(tls)?;
                    let tls_stream = pg_proto::tls::accept(
                        tcp_stream,
                        std::sync::Arc::new(server_config),
                        &leaf,
                    )
                    .await?;
                    client_is_tls = true;
                    stream = NetworkStream::server_tls(tls_stream);
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
            PreStartupOffer::Startup { conn, message } => break (conn, message),
            PreStartupOffer::Gss(conn) => {
                conn.into_transport();
                return Err(ProtocolError::UnexpectedStartupMessage.into());
            }
        }
    };

    let (mut database_startup, startup_packet) =
        Conn::new(Buffered::<_, BackendDirection>::new(database_stream))
            .startup(&startup_message)?;
    database_startup.push_startup_packet(&startup_packet);
    database_startup.flush().await?;
    let database_startup =
        authenticate_upstream(database_startup, &context, &mut upstream_middleware).await?;

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
        let (password_state, message) = match receive_frontend_auth(
            password_state,
            &mut downstream_middleware,
            connection_timeout,
        )
        .await
        {
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

    if context.require_tls() && !client_is_tls {
        let message = ErrorResponse::tls_required();
        send_backend_message(&mut client_stream, message.into_backend_message()).await?;

        error!(msg = "Client must connect with Transport Layer Security (TLS)");
        return Err(ConfigError::TlsRequired.into());
    }

    let database_stream = drain_upstream_startup(
        database_startup,
        &mut client_stream,
        &mut upstream_middleware,
    )
    .await?;

    let (client_reader, client_writer) = client_stream.into_inner().split();
    let (server_reader, server_writer) = database_stream.into_inner().split();

    let channel_writer = ChannelWriter::new(client_writer, client_id);

    let mut client_reader: Buffered<_, FrontendDirection> = Buffered::new_frontend(client_reader);
    let mut server_writer: Buffered<_, BackendDirection> = Buffered::new(server_writer);
    let mut server_reader: Buffered<_, BackendDirection> = Buffered::new(server_reader);
    let mut frontend = Middleware::new(
        FrontendDisposition::Forward,
        Frontend::new(channel_writer.sender(), context.clone()),
    );
    let mut backend = Middleware::new(
        BackendDisposition::Emit,
        Backend::new(channel_writer.sender(), context.clone()),
    );

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
    let client_context = context.clone();
    let mut backend_context = context.clone();
    let mut server_write_context = context.clone();

    let client_to_server = async {
        loop {
            let message = match receive_frontend_runtime(
                &mut client_reader,
                client_context.connection_timeout(),
            )
            .await
            {
                Ok(message) => message,
                Err(Error::ConnectionClosed) => {
                    write_to_server(
                        &mut server_writer,
                        &mut server_write_context,
                        FrontendMessage::Terminate,
                    )
                    .await?;
                    return Ok::<(), Error>(());
                }
                Err(error) => return Err(error),
            };

            let frame = message.to_frame()?;
            counter!(CLIENTS_BYTES_RECEIVED_TOTAL).increment((frame.body.len() + 5) as u64);

            let tracking_message = message.clone();
            *frontend.state_mut() = FrontendDisposition::Forward;
            let outbound = frontend.intercept(message).await?;
            if *frontend.state() == FrontendDisposition::Forward {
                client_context
                    .protocol_frontend_received(
                        tracking_message,
                        pg_proto::pipeline::FrontendHandling::Forward,
                    )
                    .await?;
                write_to_server(&mut server_writer, &mut server_write_context, outbound).await?;
            }
        }
    };

    let server_to_client = async {
        loop {
            let read_start = Instant::now();
            let message =
                receive_backend_runtime(&mut server_reader, backend_context.connection_timeout())
                    .await?;
            if server_reader.project_backend(message.clone()).is_none() {
                let _ = server_reader.demux_mut().pop_async_event();
            }
            let read_duration = read_start.elapsed();
            backend_context.record_execute_server_timing(read_duration);
            let frame = message.to_frame()?;
            counter!(SERVER_BYTES_RECEIVED_TOTAL).increment((frame.body.len() + 5) as u64);
            if read_duration > backend_context.slow_db_response_min_duration() {
                warn!(
                    client_id = backend_context.client_id,
                    msg = "Slow database response",
                    duration_ms = read_duration.as_millis(),
                    message = ?message,
                );
            }

            *backend.state_mut() = BackendDisposition::Emit;
            let outbound = backend.intercept(message).await?;
            if *backend.state() == BackendDisposition::Emit {
                backend.handler_mut().write_with_flush(outbound).await?;
            }
        }
        #[allow(unreachable_code)]
        Ok::<(), Error>(())
    };

    // Run frontend and backend tasks
    let result = tokio::try_join!(client_to_server, server_to_client);

    if let Err(ref err @ Error::ConnectionTimeout { .. }) = &result {
        let error_response = ErrorResponse::connection_timeout(err.to_string());
        let _ = timeout_sender.send(error_response.into_backend_message());
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

/// Applies CipherStash's upstream TLS policy while pg-proto owns negotiation.
async fn connect_upstream_tls(
    stream: NetworkStream<TcpStream>,
    config: &TandemConfig,
) -> Result<NetworkStream<TcpStream>, Error> {
    if config.database_tls_disabled() {
        warn!(msg = "Connecting to database without Transport Layer Security (TLS)");
        return Ok(stream);
    }

    let mut request = Conn::new(Buffered::<_, BackendDirection>::new(stream)).request_ssl();
    request.flush().await?;
    let mut middleware = Middleware::new((), Identity);
    let reply = request
        .receive_encryption_reply_typed(&mut middleware)
        .await
        .map_err(|error| match error {
            TypedReceiveError::Io(error) => Error::from(error),
            TypedReceiveError::Illegal(reply) | TypedReceiveError::InvalidWire(reply) => {
                Error::from(std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("invalid PostgreSQL encryption reply: {reply:?}"),
                ))
            }
            TypedReceiveError::Middleware(never) => match never {},
        })?;
    match request.receive_reply(reply.into_wire()) {
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

async fn receive_frontend_runtime<S: AsyncRead + Unpin>(
    reader: &mut Buffered<S, FrontendDirection>,
    connection_timeout: Option<Duration>,
) -> Result<FrontendMessage, Error> {
    match connection_timeout {
        Some(duration) => timeout(duration, reader.receive_wire())
            .await
            .map_err(|_| Error::ConnectionTimeout { duration })?
            .map_err(Into::into),
        None => reader.receive_wire().await.map_err(Into::into),
    }
}

async fn receive_backend_runtime<S: AsyncRead + Unpin>(
    reader: &mut Buffered<S, BackendDirection>,
    connection_timeout: Option<Duration>,
) -> Result<BackendMessage, Error> {
    match connection_timeout {
        Some(duration) => timeout(duration, reader.receive_backend())
            .await
            .map_err(|_| Error::ConnectionTimeout { duration })?
            .map_err(Into::into),
        None => reader.receive_backend().await.map_err(Into::into),
    }
}

async fn write_to_server<S: AsyncWrite + Unpin, E>(
    writer: &mut Buffered<S, BackendDirection>,
    context: &mut Context<E>,
    message: FrontendMessage,
) -> Result<(), Error>
where
    E: crate::proxy::EncryptionService,
{
    debug!(target: crate::log::PROTOCOL, msg = "Write to server", ?message);
    let frame = message.to_frame()?;
    counter!(SERVER_BYTES_SENT_TOTAL).increment((frame.body.len() + 5) as u64);
    let start = Instant::now();
    writer.push(frame)?;
    writer.flush().await?;
    if let Some(session_id) = context.latest_session_id() {
        context.add_server_write_duration(session_id, start.elapsed());
    }
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
