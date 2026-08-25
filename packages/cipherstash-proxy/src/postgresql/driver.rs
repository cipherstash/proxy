use super::{middleware::CipherStashMiddlewareFactory, Context};
use crate::{
    connect,
    error::{ConfigError, Error},
    proxy::EncryptionService,
    tls,
};
use pg_proto::{
    BackendForwarding, BoundedPipeline, CancellationPolicy, Client, ClientTlsConfig,
    ClientTlsPolicy, ClientTlsProvider, ConnectTarget, ForwardedMessage, FrontendForwarding,
    FrontendMessage, InMemoryCancellationRegistry, InitialServerContext, Intermediary, Server,
    ServerIdentity, ServerIdentityProvider, ServerTlsPolicy, SslMode, StartupParameters,
    StartupRouteResolver, StaticClientCredentials, StaticMd5ServerCredentials,
};
use std::{convert::Infallible, sync::Arc};
use tokio::net::TcpStream;
use tracing::info;

#[derive(Clone)]
struct Route(String);

impl<Peer: Sync> StartupRouteResolver<Peer> for Route {
    type Error = Infallible;
    async fn resolve(
        &self,
        _: StartupParameters,
        _: InitialServerContext<'_, Peer>,
    ) -> Result<ConnectTarget, Self::Error> {
        Ok(ConnectTarget::new(self.0.clone()))
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

#[derive(Clone)]
struct UpstreamTls {
    server_name: rustls_pki_types::ServerName<'static>,
    roots: Arc<rustls::RootCertStore>,
}
impl ClientTlsProvider for UpstreamTls {
    type Error = Error;
    async fn resolve(&self, _: &ConnectTarget) -> Result<ClientTlsConfig, Error> {
        Ok(ClientTlsConfig::new(
            self.server_name.clone(),
            (*self.roots).clone(),
        ))
    }
}

pub async fn handler<S>(client_stream: TcpStream, context: Context<S>) -> Result<(), Error>
where
    S: EncryptionService + Clone,
{
    validate_downstream_tls(context.require_tls(), context.tls_config().is_some())?;
    let address = context.database_socket_address();
    let downstream_auth = StaticMd5ServerCredentials::new(
        context.database_username().to_owned(),
        context.database_password(),
    );
    let upstream_auth = StaticClientCredentials::new(
        context.database_username().to_owned(),
        context.database_password(),
    );

    macro_rules! run {
        ($server:expr, $client:expr) => {{
            let connection_timeout = context.connection_timeout();
            let intermediary = Intermediary::builder()
                .server($server)
                .client($client)
                .startup_resolver(Route(address.clone()))
                .cancellation(CancellationPolicy::Forward)
                .cancellation_registry(InMemoryCancellationRegistry::default())
                .pipeline(BoundedPipeline::new(256).expect("non-zero pipeline bound"))
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
            'session: loop {
                let forward = session.forward_next();
                let forwarded = match connection_timeout {
                    Some(duration) => tokio::time::timeout(duration, forward)
                        .await
                        .map_err(|_| Error::ConnectionTimeout { duration })?,
                    None => forward.await,
                };
                let forwarded = match forwarded {
                    Ok(forwarded) => forwarded,
                    Err(pg_proto::ForwardError::Frontend(
                        pg_proto::FrontendProjectionError::Capacity(_),
                    )) => {
                        // pg-proto retains the rejected frontend message. Drain
                        // one upstream response at a time to release pipeline
                        // capacity, then retry that exact message through the
                        // dedicated frontend path. In particular, this lets a
                        // retained Sync reach PostgreSQL instead of waiting for
                        // responses that PostgreSQL may buffer until Sync.
                        loop {
                            let backend = session.forward_backend();
                            let drained = match connection_timeout {
                                Some(duration) => tokio::time::timeout(duration, backend)
                                    .await
                                    .map_err(|_| Error::ConnectionTimeout { duration })?,
                                None => backend.await,
                            };
                            match drained {
                                Ok(
                                    BackendForwarding::Forwarded(_)
                                    | BackendForwarding::Expanded { .. }
                                    | BackendForwarding::Suppressed(_)
                                    | BackendForwarding::Held,
                                ) => {}
                                Err(pg_proto::ForwardError::Middleware(error)) => {
                                    return Err(error)
                                }
                                Err(error) => return Err(invalid_data(error)),
                            }

                            let retry = session.forward_frontend();
                            let retried = match connection_timeout {
                                Some(duration) => tokio::time::timeout(duration, retry)
                                    .await
                                    .map_err(|_| Error::ConnectionTimeout { duration })?,
                                None => retry.await,
                            };
                            match retried {
                                Ok(FrontendForwarding::Forwarded(FrontendMessage::Terminate)) => {
                                    break 'session;
                                }
                                Ok(_) => break,
                                Err(pg_proto::ForwardError::Frontend(
                                    pg_proto::FrontendProjectionError::Capacity(_),
                                )) => {}
                                Err(pg_proto::ForwardError::Middleware(error)) => {
                                    return Err(error)
                                }
                                Err(error) => return Err(invalid_data(error)),
                            }
                        }
                        continue;
                    }
                    Err(pg_proto::ForwardError::Middleware(error)) => return Err(error),
                    Err(error) => return Err(invalid_data(error)),
                };
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
                    roots: context.upstream_tls_roots(),
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
                .tls(ServerTlsPolicy::Required(identity))
                .authentication(downstream_auth)
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

fn validate_downstream_tls(require_tls: bool, tls_configured: bool) -> Result<(), Error> {
    if require_tls && !tls_configured {
        return Err(ConfigError::TlsConfigurationRequired.into());
    }
    Ok(())
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upstream_tls_without_verification_is_opportunistic() {
        let config = crate::TandemConfig::for_testing();
        assert_eq!(upstream_ssl_mode(&config), SslMode::Prefer);
    }

    #[test]
    fn required_downstream_tls_rejects_missing_tls_configuration() {
        assert!(matches!(
            validate_downstream_tls(true, false),
            Err(Error::Config(ConfigError::TlsConfigurationRequired))
        ));
    }
}
