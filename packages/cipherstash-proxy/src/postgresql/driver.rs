use super::{diagnostics::ErrorResponse, middleware::CipherStashMiddlewareFactory, Context};
use crate::{
    connect,
    error::{Error, ProtocolError},
    proxy::EncryptionService,
    tls,
};
use bytes::Bytes;
use md5::{Digest, Md5};
use pg_proto::{
    Authentication, BackendHoldLimits, BackendMessage, BoundedPipeline, CancelKey,
    CancellationPolicy, CancellationRoute, Client, ClientAuthentication,
    ClientAuthenticationChallenge, ClientAuthenticationResponse, ClientAuthenticationSession,
    ClientTlsConfig, ClientTlsPolicy, ClientTlsProvider, ConnectTarget, ForwardedMessage,
    FrontendMessage, InitialServerContext, Intermediary, IntermediaryCancellationRegistry,
    MiddlewareFactory, NegotiatedServerTls, Server, ServerAuthentication,
    ServerAuthenticationAction, ServerAuthenticationFuture, ServerAuthenticationProvider,
    ServerAuthenticationRequest, ServerAuthenticationResponse, ServerConnectionContext,
    ServerIdentity, ServerIdentityProvider, ServerMiddleware, ServerTlsPolicy, SslMode,
    StartupParameters, StartupRouteResolver,
};
use postgres_protocol::authentication::sasl::{ChannelBinding, ScramSha256};
use rand::Rng;
use std::{
    collections::HashMap,
    convert::Infallible,
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex, OnceLock},
};
use tokio::net::TcpStream;
use tracing::info;

#[derive(Clone)]
struct Route(String);

#[derive(Clone, Copy)]
struct CancellationRegistry;

fn cancellation_routes() -> &'static Mutex<HashMap<CancelKey, CancellationRoute>> {
    static ROUTES: OnceLock<Mutex<HashMap<CancelKey, CancellationRoute>>> = OnceLock::new();
    ROUTES.get_or_init(|| Mutex::new(HashMap::new()))
}

impl IntermediaryCancellationRegistry for CancellationRegistry {
    type Error = std::io::Error;

    fn register(&self, route: CancellationRoute) -> Result<CancelKey, Self::Error> {
        // Keep the database-issued key unchanged. The proxy has one upstream per
        // downstream connection, so no pool-level key translation is required.
        let client_key = route.upstream_key().clone();
        let mut routes = cancellation_routes()
            .lock()
            .map_err(|_| std::io::Error::other("cancellation registry lock poisoned"))?;
        if routes.contains_key(&client_key) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "duplicate PostgreSQL cancellation key",
            ));
        }
        routes.insert(client_key.clone(), route);
        Ok(client_key)
    }

    fn resolve(&self, client: &CancelKey) -> Option<CancellationRoute> {
        cancellation_routes().lock().ok()?.get(client).cloned()
    }

    fn detach(&self, client: &CancelKey) -> Option<CancellationRoute> {
        cancellation_routes().lock().ok()?.remove(client)
    }
}

impl<Peer> StartupRouteResolver<Peer> for Route {
    type Error = Infallible;
    fn resolve<'a>(
        &'a self,
        _: StartupParameters,
        _: InitialServerContext<'a, Peer>,
    ) -> Pin<Box<dyn Future<Output = Result<ConnectTarget, Self::Error>> + 'a>> {
        let address = self.0.clone();
        Box::pin(async move { Ok(ConnectTarget::new(address)) })
    }
}

#[derive(Clone)]
struct DownstreamAuth {
    username: String,
    password: String,
}

struct DownstreamSession {
    username: String,
    password: String,
    salt: [u8; 4],
}

impl ServerAuthenticationProvider for DownstreamAuth {
    type Authentication = DownstreamSession;
    fn create(&self) -> Self::Authentication {
        let mut salt = [0; 4];
        rand::rng().fill(&mut salt);
        DownstreamSession {
            username: self.username.clone(),
            password: self.password.clone(),
            salt,
        }
    }
}

impl<Peer> ServerAuthentication<Peer> for DownstreamSession {
    type Identity = ();
    type Error = Error;
    fn start<'a>(
        &'a mut self,
        _: ServerAuthenticationRequest<'a, Peer>,
    ) -> ServerAuthenticationFuture<'a, ServerAuthenticationAction<()>, Error> {
        let salt = self.salt;
        Box::pin(async move { Ok(ServerAuthenticationAction::Md5Password { salt }) })
    }
    fn respond<'a>(
        &'a mut self,
        _: ServerAuthenticationRequest<'a, Peer>,
        response: ServerAuthenticationResponse,
    ) -> ServerAuthenticationFuture<'a, ServerAuthenticationAction<()>, Error> {
        Box::pin(async move {
            let ServerAuthenticationResponse::Password(received) = response else {
                return Err(ProtocolError::AuthenticationFailed.into());
            };
            let expected = md5_hash(
                self.username.as_bytes(),
                self.password.as_bytes(),
                &self.salt,
            );
            if received.as_ref() != expected.as_bytes() {
                return Err(ProtocolError::ClientAuthenticationFailed.into());
            }
            Ok(ServerAuthenticationAction::Accept(()))
        })
    }
}

#[derive(Clone)]
struct UpstreamAuth {
    username: String,
    password: String,
}

struct UpstreamSession {
    username: String,
    password: String,
    scram: Option<ScramSha256>,
}

impl ClientAuthentication for UpstreamAuth {
    type Evidence = ();
    type Session = UpstreamSession;
    type Error = Error;
    fn begin<'a>(
        &'a self,
        _: &'a ConnectTarget,
    ) -> Pin<Box<dyn Future<Output = Result<UpstreamSession, Error>> + 'a>> {
        let session = UpstreamSession {
            username: self.username.clone(),
            password: self.password.clone(),
            scram: None,
        };
        Box::pin(async move { Ok(session) })
    }
}

impl ClientAuthenticationSession for UpstreamSession {
    type Evidence = ();
    type Error = Error;
    fn respond<'a>(
        &'a mut self,
        challenge: ClientAuthenticationChallenge,
    ) -> Pin<Box<dyn Future<Output = Result<ClientAuthenticationResponse, Error>> + 'a>> {
        Box::pin(async move {
            match challenge {
                ClientAuthenticationChallenge::CleartextPassword => {
                    Ok(ClientAuthenticationResponse::Password(
                        Bytes::copy_from_slice(self.password.as_bytes()),
                    ))
                }
                ClientAuthenticationChallenge::Md5Password(salt) => {
                    Ok(ClientAuthenticationResponse::Password(Bytes::from(
                        md5_hash(self.username.as_bytes(), self.password.as_bytes(), &salt),
                    )))
                }
                ClientAuthenticationChallenge::Sasl(mechanisms) => {
                    if !mechanisms.iter().any(|m| m.as_ref() == b"SCRAM-SHA-256") {
                        return Err(
                            ProtocolError::UnsupportedAuthentication { method_code: -1 }.into()
                        );
                    }
                    let scram = self.scram.insert(ScramSha256::new(
                        self.password.as_bytes(),
                        ChannelBinding::unsupported(),
                    ));
                    Ok(ClientAuthenticationResponse::SaslInitial {
                        mechanism: Bytes::from_static(b"SCRAM-SHA-256"),
                        response: Bytes::copy_from_slice(scram.message()),
                    })
                }
                ClientAuthenticationChallenge::SaslContinue(challenge) => {
                    let scram = self
                        .scram
                        .as_mut()
                        .ok_or(ProtocolError::AuthenticationFailed)?;
                    scram.update(&challenge)?;
                    Ok(ClientAuthenticationResponse::Sasl(Bytes::copy_from_slice(
                        scram.message(),
                    )))
                }
                ClientAuthenticationChallenge::SaslFinal(final_message) => {
                    let scram = self
                        .scram
                        .as_mut()
                        .ok_or(ProtocolError::AuthenticationFailed)?;
                    scram.finish(&final_message)?;
                    Ok(ClientAuthenticationResponse::Verified)
                }
                _ => Err(ProtocolError::UnsupportedAuthentication { method_code: -1 }.into()),
            }
        })
    }
    fn authenticated(self) -> Pin<Box<dyn Future<Output = Result<(), Error>>>> {
        Box::pin(async { Ok(()) })
    }
}

#[derive(Clone)]
struct DownstreamIdentity(ServerIdentity);
impl ServerIdentityProvider for DownstreamIdentity {
    type Error = Infallible;
    fn resolve(&self) -> Result<ServerIdentity, Infallible> {
        Ok(self.0.clone())
    }
}

#[derive(Clone, Copy)]
struct RequireTlsDiagnostic;

impl<Peer, Identity> MiddlewareFactory<ServerConnectionContext<Peer, Identity>>
    for RequireTlsDiagnostic
{
    type Handler = Self;

    fn create(&self, _: &ServerConnectionContext<Peer, Identity>) -> Self::Handler {
        *self
    }
}

impl<State, Peer, Identity> ServerMiddleware<State, ServerConnectionContext<Peer, Identity>>
    for RequireTlsDiagnostic
{
    fn backend(
        &mut self,
        context: &ServerConnectionContext<Peer, Identity>,
        _: &mut State,
        message: BackendMessage,
    ) -> BackendMessage {
        require_tls_diagnostic(context.tls(), message)
    }
}

fn require_tls_diagnostic(tls: &NegotiatedServerTls, message: BackendMessage) -> BackendMessage {
    if matches!(tls, NegotiatedServerTls::Plaintext)
        && matches!(
            message,
            BackendMessage::Authentication(Authentication::Md5Password { .. })
        )
    {
        ErrorResponse::tls_required().into_backend_message()
    } else {
        message
    }
}

#[derive(Clone)]
struct UpstreamTls {
    server_name: rustls_pki_types::ServerName<'static>,
}
impl ClientTlsProvider for UpstreamTls {
    type Error = Error;
    fn resolve<'a>(
        &'a self,
        _: &'a ConnectTarget,
    ) -> Pin<Box<dyn Future<Output = Result<ClientTlsConfig, Error>> + 'a>> {
        let name = self.server_name.clone();
        Box::pin(async move {
            let result = rustls_native_certs::load_native_certs();
            let mut roots = rustls::RootCertStore::empty();
            for certificate in result.certs {
                roots
                    .add(certificate)
                    .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
            }
            Ok(ClientTlsConfig::new(name, roots))
        })
    }
}

pub async fn handler<S>(client_stream: TcpStream, context: Context<S>) -> Result<(), Error>
where
    S: EncryptionService + Clone,
{
    let address = context.database_socket_address();
    let downstream_auth = DownstreamAuth {
        username: context.database_username().to_owned(),
        password: context.database_password(),
    };
    let upstream_auth = UpstreamAuth {
        username: context.database_username().to_owned(),
        password: context.database_password(),
    };

    macro_rules! run {
        ($server:expr, $client:expr) => {{
            let connection_timeout = context.connection_timeout();
            let intermediary = Intermediary::builder()
                .server($server)
                .client($client)
                .startup_resolver(Route(address.clone()))
                .cancellation(CancellationPolicy::Forward)
                .cancellation_registry(CancellationRegistry)
                .pipeline(BoundedPipeline::new(256).expect("non-zero pipeline bound"))
                .backend_batching(
                    BackendHoldLimits::new(4096, 64 * 1024 * 1024)
                        .expect("non-zero backend hold limits"),
                )
                .middleware(CipherStashMiddlewareFactory(context.clone()))
                .build()
                .map_err(invalid_data)?;
            let accept = intermediary.accept(client_stream, (), ());
            let accepted = match connection_timeout {
                Some(duration) => tokio::time::timeout(duration, accept)
                    .await
                    .map_err(|_| Error::ConnectionTimeout { duration })?,
                None => accept.await,
            }
            .map_err(invalid_data)?;
            let mut session = match accepted {
                pg_proto::IntermediaryAccept::Session(session) => session,
                pg_proto::IntermediaryAccept::CancellationForwarded => return Ok(()),
            };
            loop {
                let forward = session.forward_next();
                let forwarded = match connection_timeout {
                    Some(duration) => tokio::time::timeout(duration, forward)
                        .await
                        .map_err(|_| Error::ConnectionTimeout { duration })?,
                    None => forward.await,
                }
                .map_err(invalid_data)?;
                if matches!(
                    forwarded,
                    ForwardedMessage::Frontend(FrontendMessage::Terminate)
                ) {
                    break;
                }
            }
            Ok(())
        }};
    }

    macro_rules! run_client {
        ($server:expr) => {{
            if context.database_tls_disabled() {
                let client = Client::builder()
                    .connector(|target: &ConnectTarget| {
                        let address = target.name().to_owned();
                        async move { connect::connect(&address).await }
                    })
                    .tls(ClientTlsPolicy::Disabled)
                    .authentication(upstream_auth.clone())
                    .build()
                    .map_err(invalid_data)?;
                run!($server, client)
            } else {
                let provider = UpstreamTls {
                    server_name: context.config().database.server_name()?.to_owned(),
                };
                let mode = upstream_ssl_mode(context.config());
                let client = Client::builder()
                    .connector(|target: &ConnectTarget| {
                        let address = target.name().to_owned();
                        async move { connect::connect(&address).await }
                    })
                    .tls(ClientTlsPolicy::libpq(mode, provider))
                    .authentication(upstream_auth.clone())
                    .build()
                    .map_err(invalid_data)?;
                run!($server, client)
            }
        }};
    }

    info!(
        msg = "Client connected",
        database = address,
        client_id = context.client_id
    );
    if let Some(tls_config) = context.tls_config() {
        let (config, leaf) = tls::configure_server_with_leaf(tls_config)?;
        let identity = DownstreamIdentity(ServerIdentity::new(Arc::new(config), leaf));
        if context.require_tls() {
            return run_client!(Server::builder()
                .tls(ServerTlsPolicy::Optional(identity))
                .authentication(downstream_auth)
                .middleware(RequireTlsDiagnostic)
                .build()
                .map_err(invalid_data)?);
        }
        return run_client!(Server::builder()
            .tls(ServerTlsPolicy::Optional(identity))
            .authentication(downstream_auth)
            .build()
            .map_err(invalid_data)?);
    }

    run_client!(Server::builder()
        .tls(ServerTlsPolicy::Disabled)
        .authentication(downstream_auth)
        .build()
        .map_err(invalid_data)?)
}

fn invalid_data(error: impl std::fmt::Display) -> Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, error.to_string()).into()
}

fn upstream_ssl_mode(config: &crate::TandemConfig) -> SslMode {
    if config.database.with_tls_verification {
        SslMode::VerifyFull
    } else {
        // Preserve the proxy's historical opportunistic-TLS policy: attempt
        // SSL, but continue in plaintext when the database rejects it.
        SslMode::Prefer
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_tls_without_verification_is_opportunistic() {
        let config = crate::TandemConfig::for_testing();
        assert_eq!(upstream_ssl_mode(&config), SslMode::Prefer);
    }

    #[test]
    fn required_tls_plaintext_connection_receives_diagnostic() {
        let challenge =
            BackendMessage::Authentication(Authentication::Md5Password { salt: [1, 2, 3, 4] });
        assert!(matches!(
            require_tls_diagnostic(&NegotiatedServerTls::Plaintext, challenge),
            BackendMessage::ErrorResponse(_)
        ));
    }

    #[test]
    fn required_tls_encrypted_connection_authenticates_normally() {
        let challenge =
            BackendMessage::Authentication(Authentication::Md5Password { salt: [1, 2, 3, 4] });
        let tls = NegotiatedServerTls::Tls {
            server_end_point: Bytes::new(),
        };
        assert_eq!(require_tls_diagnostic(&tls, challenge.clone()), challenge);
    }
}
