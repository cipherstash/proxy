use super::tls::TlsConfig;
use super::{
    DatabaseConfig, LogConfig, LogLevel, ServerConfig, CS_PREFIX, DEBUG_THREAD_STACK_SIZE,
    DEFAULT_CONFIG_FILE_PATH, DEFAULT_THREAD_STACK_SIZE,
};
use crate::config::LogFormat;
use crate::error::{ConfigError, Error};
use crate::Args;
use cipherstash_client::config::vars::{
    CS_CLIENT_ACCESS_KEY, CS_CLIENT_ID, CS_CLIENT_KEY, CS_DEFAULT_KEYSET_ID, CS_WORKSPACE_CRN,
};
use cipherstash_client::zerokms::ClientKey;
use config::{Config, Environment};
use cts_common::Crn;
use regex::Regex;
use serde::Deserialize;
use std::collections::HashMap;
use std::env;
use std::sync::LazyLock;
use uuid::Uuid;

#[derive(Clone, Debug, Deserialize)]
pub struct TandemConfig {
    #[serde(default)]
    pub server: ServerConfig,
    pub database: DatabaseConfig,
    pub auth: AuthConfig,
    pub encrypt: EncryptConfig,
    pub tls: Option<TlsConfig>,
    #[serde(default)]
    pub log: LogConfig,
    #[serde(default)]
    pub prometheus: PrometheusConfig,
    pub development: Option<DevelopmentConfig>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct AuthConfig {
    pub workspace_crn: Crn,
    pub client_access_key: String,
}

#[derive(Debug, Deserialize, Clone, PartialEq)]
pub struct EncryptConfig {
    pub client_id: Uuid,
    pub client_key: String,
    pub default_keyset_id: Option<Uuid>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct PrometheusConfig {
    #[serde(default)]
    pub enabled: bool,

    #[serde(default = "PrometheusConfig::default_port")]
    pub port: u16,
}

#[derive(Clone, Debug, Deserialize)]
pub struct DevelopmentConfig {
    #[serde(default)]
    pub disable_mapping: bool,

    #[serde(default)]
    pub disable_database_tls: bool,

    #[serde(default)]
    pub enable_mapping_errors: bool,
}

/// The outcome of validating the inbound (client-facing) TLS configuration.
#[derive(Clone, Debug, PartialEq)]
pub enum InboundTls {
    /// TLS is configured and valid.
    Enabled,
    /// TLS is not configured, so the proxy listens in plaintext.
    NotConfigured,
}

impl InboundTls {
    /// A notice for the user, printed regardless of log level.
    /// `no_tls` is true when plaintext was explicitly requested with --no-tls.
    pub fn notice(&self, no_tls: bool) -> Option<String> {
        match self {
            InboundTls::Enabled => None,
            InboundTls::NotConfigured if no_tls => None,
            InboundTls::NotConfigured => Some(
                "TLS is not configured; listening for client connections in plaintext. \
                 Pass --no-tls to silence this warning."
                    .to_string(),
            ),
        }
    }
}

/// Config defaults to a file called `tandem` in the current directory.
/// Supports TOML, JSON, YAML
/// Variable names should match the struct field names.
///
/// ENV vars can be used to override file settings.
///
/// ENV vars must be prefixed with `CS_`.
///
/// IMPORTANT!
/// Config needs to be loaded *before* logging can be initalized
/// Tracing macros will not output and you may be confused.
/// Use `println!`` and `eprintln!`` instead.
///
impl TandemConfig {
    pub fn default_path() -> String {
        DEFAULT_CONFIG_FILE_PATH.to_string()
    }

    pub fn load(args: &Args) -> Result<TandemConfig, Error> {
        let mut config = TandemConfig::build(args)?;

        // If log level is default, it has not been set by the user in config
        if config.log.level == LogConfig::default_log_level() {
            if let Some(log_level) = args.log_level {
                config.log.level = log_level;
            }
        }

        // If log format is default, it has not been set by the user in config
        if config.log.format == LogConfig::default_log_format() {
            config.log.format = args.log_format;
        }

        // --no-tls forces a plaintext inbound listener, ignoring any CS_TLS__*
        // env / config. (Does not affect the proxy -> database connection.)
        if args.no_tls {
            config.tls = None;
            config.server.require_tls = false;
        }

        // --tls requires inbound TLS: clients that skip SSLRequest are rejected.
        if args.tls {
            config.server.require_tls = true;
            config.validate()?;
        }

        Ok(config)
    }

    pub fn build_path(path: &str) -> Result<Self, Error> {
        let args = Args {
            config_file_path: path.to_string(),
            database_url: None,
            db_host: None,
            db_port: None,
            db_name: None,
            db_user: None,
            db_password: None,
            no_tls: false,
            tls: false,
            debug: false,
            log_level: None,
            log_format: LogConfig::default_log_format(),
            command: None,
        };
        Self::build(&args)
    }

    pub fn build(args: &Args) -> Result<Self, Error> {
        // For parsing top-level values such as CS_HOST, CS_PORT
        // and for parsing nested env values such as CS_DATABASE__HOST, CS_DATABASE__PORT
        let cs_env_source = Environment::with_prefix(CS_PREFIX)
            .ignore_empty(true)
            .try_parsing(true)
            .separator("__")
            .prefix_separator("_");

        // Env vars from `stash setup --proxy`
        let stash_setup_source = Environment::with_prefix(CS_PREFIX)
            .ignore_empty(true)
            .separator("__")
            .prefix_separator("_")
            .source(Some({
                let mut env = HashMap::new();

                if let Ok(value) = std::env::var(CS_CLIENT_ID) {
                    env.insert("CS_ENCRYPT__CLIENT_ID".into(), value);
                }

                if let Ok(value) = std::env::var(CS_CLIENT_KEY) {
                    env.insert("CS_ENCRYPT__CLIENT_KEY".into(), value);
                }

                if let Ok(value) = std::env::var(CS_DEFAULT_KEYSET_ID) {
                    env.insert("CS_ENCRYPT__DEFAULT_KEYSET_ID".into(), value);
                }

                if let Ok(value) = std::env::var(CS_WORKSPACE_CRN) {
                    value
                        .parse::<Crn>()
                        .map(|crn| {
                            env.insert("CS_AUTH__WORKSPACE_CRN".into(), crn.to_string());
                        })
                        .map_err(|_| ConfigError::InvalidWorkspaceCrn { crn: value })?;
                }

                if let Ok(value) = std::env::var(CS_CLIENT_ACCESS_KEY) {
                    env.insert("CS_AUTH__CLIENT_ACCESS_KEY".into(), value);
                }

                env
            }));

        // Command line arguments override env vars.
        // A full connection string is applied first; individual flags below
        // then override matching parts of it.
        if let Some(url) = &args.database_url {
            apply_database_url(url)?;
        }

        if let Some(db_host) = &args.db_host {
            env::set_var("CS_DATABASE__HOST", db_host);
        }

        if let Some(db_port) = &args.db_port {
            env::set_var("CS_DATABASE__PORT", db_port.to_string());
        }

        if let Some(dbname) = &args.db_name {
            env::set_var("CS_DATABASE__NAME", dbname);
        }

        if let Some(db_user) = &args.db_user {
            env::set_var("CS_DATABASE__USERNAME", db_user);
        }

        if let Some(db_password) = &args.db_password {
            env::set_var("CS_DATABASE__PASSWORD", db_password);
        }

        // Source order is important!
        let config: TandemConfig = Config::builder()
            .add_source(config::File::with_name(&args.config_file_path).required(false))
            .add_source(cs_env_source)
            .add_source(stash_setup_source)
            .build()?
            .try_deserialize()
            .map_err(|err| {
                // ConfigError is not helping here
                //  - does not carry the information in structured form
                //  - missing parameters are returned by at least two different errors, depending the source of the error
                // Easier to inspect the error message.
                match err.to_string() {
                    s if s.contains("UUID parsing failed") => {
                        if s.contains("client_id") && !s.contains("keyset") {
                            ConfigError::InvalidParameter {
                                name: "client_id".to_string(),
                                value: "invalid UUID".to_string(),
                            }
                        } else {
                            ConfigError::InvalidDefaultKeysetId
                        }
                    }
                    s if s.contains("missing field") => {
                        let (field, key) = extract_missing_field_and_key(&s);
                        match (field, key) {
                            (field, None) if field == "auth" => ConfigError::MissingAuthKey,
                            (field, None) if field == "encrypt" => ConfigError::MissingEncryptKey,
                            (field, None) if field == "database" => ConfigError::MissingDatabaseKey,
                            (field, None) => ConfigError::MissingField { field },
                            (field, Some(key)) => ConfigError::MissingFieldForKey { key, field },
                        }
                    }
                    s if s.contains("does not have variant constructor") => {
                        let (name, value) = extract_invalid_field(&s);
                        ConfigError::InvalidParameter { name, value }
                    }
                    _ => err.into(),
                }
            })?;

        config.validate()?;
        config.encrypt.build_client_key()?;

        Ok(config)
    }

    fn validate(&self) -> Result<(), Error> {
        if self.server.require_tls && self.tls.is_none() {
            return Err(ConfigError::TlsConfigurationRequired.into());
        }
        Ok(())
    }

    /// Validate the inbound (client-facing) TLS configuration.
    ///
    /// TLS that is configured but invalid is fatal, at startup and on reload:
    /// a plaintext fallback would silently downgrade clients that use
    /// `sslmode=prefer`. Without TLS configuration, the proxy listens in
    /// plaintext unless `require_tls` is set (by `--tls` or
    /// `CS_SERVER__REQUIRE_TLS`).
    pub fn resolve_inbound_tls(&self) -> Result<InboundTls, Error> {
        match &self.tls {
            Some(tls) => {
                tls.check_cert().map_err(ConfigError::from)?;
                tls.check_private_key().map_err(ConfigError::from)?;
                crate::tls::configure_server(tls)?;
                Ok(InboundTls::Enabled)
            }
            None if self.server.require_tls => Err(ConfigError::TlsConfigurationRequired.into()),
            None => Ok(InboundTls::NotConfigured),
        }
    }

    pub fn database_tls_disabled(&self) -> bool {
        match &self.development {
            Some(dev) => dev.disable_database_tls,
            None => false,
        }
    }

    pub fn mapping_disabled(&self) -> bool {
        match &self.development {
            Some(dev) => dev.disable_mapping,
            None => false,
        }
    }

    pub fn mapping_errors_enabled(&self) -> bool {
        match &self.development {
            Some(dev) => dev.enable_mapping_errors,
            None => false,
        }
    }

    pub fn use_structured_logging(&self) -> bool {
        matches!(self.log.format, LogFormat::Structured)
    }

    ///
    /// Returns true if Prometheus export is enabled
    ///
    pub fn prometheus_enabled(&self) -> bool {
        self.prometheus.enabled
    }

    ///
    /// Thread stack size
    /// Not defined using a default, as we depend on the log level to increase the size for debugging
    ///
    /// In order of precedence
    ///     config if explicitly set
    ///     RUST_MIN_STACK env var if set
    ///     DEBUG_THREAD_STACK_SIZE for debug builds or Debug/Trace logging
    ///     otherwise set to DEFAULT_THREAD_STACK_SIZE (2MiB)
    ///
    pub fn thread_stack_size(&self) -> usize {
        // If the config variable is set, use that value
        if let Some(thread_stack_size) = self.server.thread_stack_size {
            return thread_stack_size;
        }

        // If the environment variable is set, use that value
        if let Ok(stack_size) = env::var("RUST_MIN_STACK") {
            return stack_size
                .parse()
                .inspect_err(|err| {
                    println!("Could not parse env var RUST_MIN_STACK: {err}");
                    println!("Using the default thread stack size");
                })
                .unwrap_or(DEFAULT_THREAD_STACK_SIZE);
        }

        if cfg!(debug_assertions)
            || self.log.level == LogLevel::Debug
            || self.log.level == LogLevel::Trace
        {
            return DEBUG_THREAD_STACK_SIZE;
        }

        DEFAULT_THREAD_STACK_SIZE
    }
    /// Returns true if slow statement logging is enabled
    pub fn slow_statements_enabled(&self) -> bool {
        self.log.slow_statements
    }

    /// Returns the slow statement minimum duration as a Duration
    pub fn slow_statement_min_duration(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.log.slow_statement_min_duration_ms)
    }

    /// Returns the slow database response minimum duration as a Duration
    pub fn slow_db_response_min_duration(&self) -> std::time::Duration {
        std::time::Duration::from_millis(self.log.slow_db_response_min_duration_ms)
    }
    #[cfg(test)]
    pub fn for_testing() -> Self {
        Self {
            server: ServerConfig::default(),
            database: DatabaseConfig::for_testing(),
            auth: AuthConfig {
                workspace_crn: "crn:ap-southeast-2.aws:IJGECSCWKREECNBS".parse().unwrap(),
                client_access_key: "test".to_string(),
            },
            encrypt: EncryptConfig {
                client_id: Uuid::parse_str("00000000-0000-0000-0000-000000000001").unwrap(),
                client_key: "a4627031a16b7065726d75746174696f6e900e05030d0608090007020c04010b0a0f6770325f66726f6da16b7065726d75746174696f6e900608000a0204030f01070d090e0b0c056570325f746fa16b7065726d75746174696f6e90000908060701030a05040e020d0b0c0f627033a16b7065726d75746174696f6e982107181d130d05181f08040a181c1002181e010311181818200b0f0e0915181b0c16171819060012181a14".to_string(),
                default_keyset_id: Some(
                    Uuid::parse_str("00000000-0000-0000-0000-000000000000").unwrap(),
                ),
            },
            tls: None,
            log: LogConfig::default(),
            prometheus: PrometheusConfig::default(),
            development: None,
        }
    }

    #[cfg(test)]
    pub fn disable_mapping_for_testing(&mut self) {
        self.development = Some(DevelopmentConfig {
            disable_mapping: true,
            disable_database_tls: false,
            enable_mapping_errors: false,
        });
    }
}

impl EncryptConfig {
    pub fn build_client_key(&self) -> Result<ClientKey, Error> {
        ClientKey::from_hex_v1(self.client_id, &self.client_key)
            .map_err(|_| Error::from(ConfigError::InvalidClientKey))
    }
}

impl PrometheusConfig {
    pub fn default_port() -> u16 {
        9930
    }
}

impl Default for PrometheusConfig {
    fn default() -> Self {
        PrometheusConfig {
            enabled: false,
            port: PrometheusConfig::default_port(),
        }
    }
}

static RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"`([^`]+)`").unwrap());

/// Parse a PostgreSQL connection string (e.g.
/// "postgres://user:pass@host:5432/dbname") and set the corresponding
/// CS_DATABASE__* env vars for each component present. Individual --db-* flags
/// are applied after this and take precedence.
fn apply_database_url(url: &str) -> Result<(), Error> {
    use std::str::FromStr;
    use tokio_postgres::config::{Host, SslMode};

    let invalid = |reason: &str| -> Error {
        ConfigError::InvalidDatabaseUrl {
            reason: reason.to_string(),
        }
        .into()
    };

    // Proxy negotiates database TLS itself (as `sslmode=prefer`), so honouring
    // only part of the URL could silently downgrade `sslmode=require`.
    const DATABASE_TLS_HINT: &str = "Proxy connects to the database with sslmode=prefer. \
        Remove sslmode and other ssl options from the URL, and set \
        CS_DATABASE__WITH_TLS_VERIFICATION=true to verify the database certificate";

    let pg = tokio_postgres::Config::from_str(url).map_err(|err| {
        // The cause names the option, as in "invalid value for option `sslmode`".
        let err =
            std::error::Error::source(&err).map_or(err.to_string(), |cause| cause.to_string());
        if err.contains("`ssl") {
            invalid(DATABASE_TLS_HINT)
        } else {
            invalid(&err)
        }
    })?;

    match pg.get_ssl_mode() {
        SslMode::Prefer => {}
        SslMode::Disable => {
            return Err(invalid(
                "sslmode=disable is not supported. Remove it from the URL, and set \
                 CS_DEVELOPMENT__DISABLE_DATABASE_TLS=true to connect to the database without TLS",
            ))
        }
        _ => return Err(invalid(DATABASE_TLS_HINT)),
    }

    let host = match pg.get_hosts() {
        [] => None,
        [Host::Tcp(host)] => Some(host.clone()),
        [_] => {
            return Err(invalid(
                "Unix socket hosts are not supported. Use a TCP host name or address",
            ))
        }
        _ => {
            return Err(invalid(
                "multiple hosts are not supported. Use a single host",
            ))
        }
    };

    let port = match pg.get_ports() {
        [] => None,
        [port] => Some(*port),
        _ => {
            return Err(invalid(
                "multiple ports are not supported. Use a single port",
            ))
        }
    };

    // Refuse every other connection option rather than silently ignoring it:
    // the URL must equal a config built from the parts Proxy applies.
    let mut applied = tokio_postgres::Config::new();
    if let Some(user) = pg.get_user() {
        applied.user(user);
    }
    if let Some(password) = pg.get_password() {
        applied.password(password);
    }
    if let Some(dbname) = pg.get_dbname() {
        applied.dbname(dbname);
    }
    if let Some(host) = &host {
        applied.host(host);
    }
    if let Some(port) = port {
        applied.port(port);
    }
    if applied != pg {
        return Err(invalid(
            "only the user, password, host, port and database name are supported. \
             Remove other connection options from the URL",
        ));
    }

    // The password is required downstream as a String. Rather than silently
    // dropping a non-UTF8 password (which would leave the proxy connecting
    // with no password, or a stale env/config one), fail clearly.
    let password = pg
        .get_password()
        .map(std::str::from_utf8)
        .transpose()
        .map_err(|_| invalid("password contains invalid UTF-8"))?;

    if let Some(host) = host {
        env::set_var("CS_DATABASE__HOST", host);
    }

    // tokio-postgres fills in 5432 for a URL without a port, which would
    // replace the configured port.
    if let Some(port) = port.filter(|_| url_has_port(url)) {
        env::set_var("CS_DATABASE__PORT", port.to_string());
    }

    if let Some(user) = pg.get_user() {
        env::set_var("CS_DATABASE__USERNAME", user);
    }

    if let Some(password) = password {
        env::set_var("CS_DATABASE__PASSWORD", password);
    }

    if let Some(dbname) = pg.get_dbname() {
        env::set_var("CS_DATABASE__NAME", dbname);
    }

    Ok(())
}

/// Whether a connection string names a port. A `postgres://` URL names one
/// in its authority, after the host; a key=value string only has ports it
/// sets.
fn url_has_port(url: &str) -> bool {
    let Some(rest) = url
        .strip_prefix("postgres://")
        .or_else(|| url.strip_prefix("postgresql://"))
    else {
        return true;
    };
    let authority = rest.split(['/', '?']).next().unwrap_or_default();
    let host_and_port = authority
        .rsplit_once('@')
        .map_or(authority, |(_, host_and_port)| host_and_port);
    match host_and_port.rsplit_once(']') {
        Some((_, after_ipv6_host)) => after_ipv6_host.starts_with(':'),
        None => host_and_port.contains(':'),
    }
}

/// Expected string is in the forms:
///     "missing field `{field}}` for key `{key}`"
///     "missing field `{field}}`"
///
fn extract_missing_field_and_key(input: &str) -> (String, Option<String>) {
    let default = "unknown";
    let values = RE
        .find_iter(input)
        .map(|m| m.as_str().trim_matches('`'))
        .collect::<Vec<_>>();
    (
        values.first().map_or(default.to_owned(), |s| s.to_string()),
        values.get(1).map(|s| s.to_string()),
    )
}

///
/// Extracts a field name (if present) from a config::ConfigError string
/// This is called in `build` if a ConfigError message contains the string `does not have variant constructor`
///
/// Error string is `enum {name} does not have variant constructor {value}`
///
fn extract_invalid_field(input: &str) -> (String, String) {
    let words = input.split(" ").collect::<Vec<_>>();

    let default_name = "unknown".to_string();
    let default_val = "".to_string();

    if !input.starts_with("enum") {
        return (default_name, default_val);
    }

    let name = words
        .get(1)
        .map_or(default_name.to_owned(), |w| w.to_string());

    let value = words
        .last()
        .map_or(default_val.to_owned(), |w| w.to_string());

    (name, value)
}

#[cfg(test)]
mod tests {
    use crate::test_helpers::with_no_cs_vars;
    use crate::{
        config::{tandem::extract_missing_field_and_key, InboundTls, TandemConfig},
        error::{ConfigError, Error},
    };
    use cipherstash_client::config::vars::{
        CS_CLIENT_ACCESS_KEY, CS_CLIENT_ID, CS_CLIENT_KEY, CS_DEFAULT_KEYSET_ID,
    };
    use std::collections::HashMap;
    use uuid::Uuid;

    const CS_PREFIX: &str = "CS_TEST";

    #[test]
    fn require_tls_needs_server_tls_configuration() {
        let mut config = TandemConfig::for_testing();
        config.server.require_tls = true;
        config.tls = None;

        assert!(matches!(
            config.validate(),
            Err(Error::Config(ConfigError::TlsConfigurationRequired))
        ));
    }

    #[test]
    /// --database-url sets each CS_DATABASE__* var, including USERNAME (not USER
    /// -- a previous bug set the wrong var so --db-user was a silent no-op).
    fn database_url_sets_connection_env() {
        use crate::config::tandem::apply_database_url;
        with_no_cs_vars(|| {
            temp_env::with_vars_unset(
                [
                    "CS_DATABASE__HOST",
                    "CS_DATABASE__PORT",
                    "CS_DATABASE__USERNAME",
                    "CS_DATABASE__USER",
                    "CS_DATABASE__PASSWORD",
                    "CS_DATABASE__NAME",
                ],
                || {
                    apply_database_url("postgres://alice:s3cret@db.example.com:5430/orders")
                        .unwrap();

                    assert_eq!(
                        std::env::var("CS_DATABASE__HOST").unwrap(),
                        "db.example.com"
                    );
                    assert_eq!(std::env::var("CS_DATABASE__PORT").unwrap(), "5430");
                    assert_eq!(std::env::var("CS_DATABASE__USERNAME").unwrap(), "alice");
                    assert_eq!(std::env::var("CS_DATABASE__PASSWORD").unwrap(), "s3cret");
                    assert_eq!(std::env::var("CS_DATABASE__NAME").unwrap(), "orders");
                    // The old (buggy) variable name must not be set.
                    assert!(std::env::var("CS_DATABASE__USER").is_err());
                },
            );
        });
    }

    #[test]
    /// A non-UTF8 password in --database-url is an error, not a silent no-op.
    /// tokio-postgres percent-decodes the password to raw bytes without UTF-8
    /// validation, so `%FF` yields invalid UTF-8; we must reject it rather than
    /// leave CS_DATABASE__PASSWORD unset (which would connect with no/stale
    /// credentials).
    fn database_url_rejects_non_utf8_password() {
        use crate::config::tandem::apply_database_url;
        with_no_cs_vars(|| {
            temp_env::with_vars_unset(["CS_DATABASE__PASSWORD"], || {
                let result = apply_database_url("postgres://alice:%FF@db.example.com/orders");
                assert!(
                    result.is_err(),
                    "expected a non-UTF8 password to be rejected"
                );
                // And it must not have set a password from the bad URL.
                assert!(std::env::var("CS_DATABASE__PASSWORD").is_err());
            });
        });
    }

    #[test]
    /// the env vars from stash setup should be the preferred option
    /// File -> extended env (generated by the config struct layout) -> stash setup env
    fn with_stash_cli_config() {
        with_no_cs_vars(|| {
            temp_env::with_vars(
                [
                    // Orignal recipe ENV var
                    (
                        "CS_ENCRYPT__CLIENT_ID",
                        Some("11111111-1111-1111-1111-111111111111"),
                    ),
                    (CS_CLIENT_ID, Some("22222222-2222-2222-2222-222222222222")),
                    (CS_CLIENT_KEY, Some("a4627031a16b7065726d75746174696f6e900e05030d0608090007020c04010b0a0f6770325f66726f6da16b7065726d75746174696f6e900608000a0204030f01070d090e0b0c056570325f746fa16b7065726d75746174696f6e90000908060701030a05040e020d0b0c0f627033a16b7065726d75746174696f6e982107181d130d05181f08040a181c1002181e010311181818200b0f0e0915181b0c16171819060012181a14")),
                    (
                        CS_DEFAULT_KEYSET_ID,
                        Some("dd0a239f-02e2-4c8e-ba20-d9f0f85af9ac"),
                    ),
                    (CS_CLIENT_ACCESS_KEY, Some("CS_CLIENT_ACCESS_KEY")),
                ],
                || {
                    let config =
                        TandemConfig::build_path("tests/config/cipherstash-proxy-test.toml")
                            .unwrap();

                    assert_eq!(
                        config.encrypt.client_id,
                        Uuid::parse_str("22222222-2222-2222-2222-222222222222").unwrap()
                    );

                    assert_eq!(
                        config.auth.client_access_key,
                        "CS_CLIENT_ACCESS_KEY".to_string()
                    );

                    assert_eq!(
                        config.encrypt.default_keyset_id,
                        Some(Uuid::parse_str("dd0a239f-02e2-4c8e-ba20-d9f0f85af9ac").unwrap())
                    );
                },
            );
        });
    }

    #[test]
    fn with_extended_env_naming() {
        with_no_cs_vars(|| {
            temp_env::with_vars(
                [
                    // Orignal recipe ENV var
                    (
                        "CS_ENCRYPT__CLIENT_ID",
                        Some("dd0a239f-02e2-4c8e-ba20-d9f0f85af9ac"),
                    ),
                    (CS_CLIENT_ID, None),
                ],
                || {
                    let config =
                        TandemConfig::build_path("tests/config/cipherstash-proxy-test.toml")
                            .unwrap();

                    assert_eq!(
                        config.encrypt.client_id,
                        Uuid::parse_str("dd0a239f-02e2-4c8e-ba20-d9f0f85af9ac").unwrap()
                    );
                },
            );
        });
    }

    #[test]
    fn database_as_url() {
        let config = with_no_cs_vars(|| {
            TandemConfig::build_path("tests/config/cipherstash-proxy-test.toml").unwrap()
        });
        assert_eq!(
            config.database.to_socket_address(),
            "localhost:5532".to_string()
        );
    }

    #[test]
    fn dataset_as_uuid() {
        temp_env::with_vars_unset(["CS_ENCRYPT__DATASET_ID", "CS_DEFAULT_KEYSET_ID"], || {
            let config = with_no_cs_vars(|| {
                TandemConfig::build_path("tests/config/cipherstash-proxy-test.toml").unwrap()
            });
            assert_eq!(
                config.encrypt.default_keyset_id,
                Some(Uuid::parse_str("484cd205-99e8-41ca-acfe-55a7e25a8ec2").unwrap())
            );

            let config =
                TandemConfig::build_path("tests/config/cipherstash-proxy-bad-dataset.toml");

            assert!(config.is_err());
            assert!(matches!(config.unwrap_err(), Error::Config(_)));
        });
    }

    #[test]
    fn invalid_client_id_uuid() {
        with_no_cs_vars(|| {
            let result =
                TandemConfig::build_path("tests/config/cipherstash-proxy-bad-client-id.toml");
            assert!(result.is_err());
            let err = result.unwrap_err();
            // Should produce InvalidParameter for client_id, not InvalidDatasetId
            assert!(
                err.to_string().contains("Invalid client_id"),
                "Expected 'Invalid client_id' but got: {}",
                err
            );
        });
    }

    #[test]
    fn prometheus_config() {
        with_no_cs_vars(|| {
            let config =
                TandemConfig::build_path("tests/config/cipherstash-proxy-test.toml").unwrap();
            assert!(!config.prometheus_enabled());

            temp_env::with_vars([("CS_PROMETHEUS__ENABLED", Some("true"))], || {
                let config =
                    TandemConfig::build_path("tests/config/cipherstash-proxy-test.toml").unwrap();
                assert!(config.prometheus_enabled());
                assert!(config.prometheus.enabled);
                assert_eq!(config.prometheus.port, 9930);
            });

            temp_env::with_vars([("CS_PROMETHEUS__PORT", Some("7777"))], || {
                let config =
                    TandemConfig::build_path("tests/config/cipherstash-proxy-test.toml").unwrap();
                assert!(!config.prometheus_enabled());
                assert!(!config.prometheus.enabled);
                assert_eq!(config.prometheus.port, 7777);
            });

            temp_env::with_vars(
                [
                    ("CS_PROMETHEUS__ENABLED", Some("true")),
                    ("CS_PROMETHEUS__PORT", Some("7777")),
                ],
                || {
                    let config =
                        TandemConfig::build_path("tests/config/cipherstash-proxy-test.toml")
                            .unwrap();
                    assert!(config.prometheus_enabled());
                    assert!(config.prometheus.enabled);
                    assert_eq!(config.prometheus.port, 7777);
                },
            );
        });
    }

    #[test]
    /// the env vars from stash setup should be the preferred option
    /// File -> extended env (generated by the config struct layout) -> stash setup env
    fn extract_field_and_key_name_from_config_error() {
        let s = "missing field `client_access_key` for key `auth`";

        let (field, key) = extract_missing_field_and_key(s);

        assert_eq!(field, "client_access_key".to_string());
        assert_eq!(key.unwrap(), "auth".to_string());

        // Bad input - auth is extracted as field
        let s = "blah {client_access_key} for vtha `auth`";

        let (field, key) = extract_missing_field_and_key(s);

        assert_eq!(field, "auth".to_string());
        assert!(key.is_none());

        // Bad input - no field can be extracted is extracted as field
        let s = "blah {client_access_key} for vtha auth";

        let (field, key) = extract_missing_field_and_key(s);

        assert_eq!(field, "unknown".to_string());
        assert!(key.is_none());
    }

    /// Returns the default environment variables as a Vec
    fn default_env_vars() -> Vec<(&'static str, Option<&'static str>)> {
        vec![
            ("CS_CLIENT_ID", Some("00000000-0000-0000-0000-000000000000")),
            ("CS_CLIENT_KEY", Some("a4627031a16b7065726d75746174696f6e900e05030d0608090007020c04010b0a0f6770325f66726f6da16b7065726d75746174696f6e900608000a0204030f01070d090e0b0c056570325f746fa16b7065726d75746174696f6e90000908060701030a05040e020d0b0c0f627033a16b7065726d75746174696f6e982107181d130d05181f08040a181c1002181e010311181818200b0f0e0915181b0c16171819060012181a14")),
            (
                "CS_DEFAULT_KEYSET_ID",
                Some("00000000-0000-0000-0000-000000000000"),
            ),
            ("CS_CLIENT_ACCESS_KEY", Some("CS_CLIENT_ACCESS_KEY")),
            ("CS_DATABASE__USERNAME", Some("CS_DATABASE__USERNAME")),
            ("CS_DATABASE__PASSWORD", Some("CS_DATABASE__PASSWORD")),
            ("CS_DATABASE__NAME", Some("CS_DATABASE__NAME")),
        ]
    }

    /// Merges the default environment variables with overrides
    fn merge_env_vars(
        overrides: Vec<(&'static str, Option<&'static str>)>,
    ) -> Vec<(&'static str, Option<&'static str>)> {
        let mut env_map: HashMap<&str, Option<&str>> = default_env_vars().into_iter().collect();

        for (key, value) in overrides {
            env_map.insert(key, value);
        }

        env_map.into_iter().collect()
    }

    // copy-pasted to tandem.rs
    #[test]
    fn with_crn_ignores_workspace_id() {
        let env = merge_env_vars(vec![(
            "CS_WORKSPACE_CRN",
            Some("crn:us-west-1.aws:E4UMRN47WJNSMAKR"),
        )]);

        with_no_cs_vars(|| {
            temp_env::with_vars(env, || {
                let config = TandemConfig::build_path("tests/config/unknown.toml").unwrap();
                assert_eq!(
                    "E4UMRN47WJNSMAKR",
                    config.auth.workspace_crn.workspace_id.to_string()
                );
            })
        });

        let env = merge_env_vars(vec![
            ("CS_WORKSPACE_ID", Some("DCMBTGHEX5R2RMR4")),
            (
                "CS_WORKSPACE_CRN",
                Some("crn:us-west-1.aws:E4UMRN47WJNSMAKR"),
            ),
        ]);

        with_no_cs_vars(|| {
            temp_env::with_vars(env, || {
                let config = TandemConfig::build_path("tests/config/unknown.toml");

                assert_eq!(
                    "E4UMRN47WJNSMAKR",
                    config.unwrap().auth.workspace_crn.workspace_id.to_string()
                );
            })
        })
    }

    #[test]
    fn no_crn_provided() {
        let env = merge_env_vars(vec![("CS_WORKSPACE_CRN", None)]);

        with_no_cs_vars(|| {
            temp_env::with_vars(env, || {
                let config = TandemConfig::build_path("tests/config/unknown.toml");
                assert!(config.is_err());

                if let Err(e) = config {
                    assert!(e
                        .to_string()
                        .contains("Missing workspace_crn from [auth] configuration. For help visit https://github.com/cipherstash/proxy/blob/main/docs/how-to/index.md#configuring-proxy"));
                }
            })
        });
    }

    #[test]
    fn invalid_crn_provided() {
        let env = merge_env_vars(vec![(
            "CS_WORKSPACE_CRN",
            Some("crn:ap-southeast-N.aws:ABCDE12345"),
        )]);

        with_no_cs_vars(|| {
            temp_env::with_vars(env, || {
                let config = TandemConfig::build_path("tests/config/unknown.toml");
                assert!(config.is_err());

                if let Err(e) = config {
                    assert!(e.to_string().contains("Invalid Workspace CRN"));
                }
            })
        });
    }

    #[test]
    fn missing_auth_config() {
        let env = merge_env_vars(vec![("CS_CLIENT_ACCESS_KEY", None)]);

        with_no_cs_vars(|| {
            temp_env::with_vars(env, || {
                let result = TandemConfig::build_path("tests/config/unknown.toml");
                assert!(result.is_err());

                if let Err(err) = result {
                    assert!(err.to_string().contains("Missing [auth] configuration"));
                } else {
                    unreachable!();
                }
            })
        });

        // Missing client_access_key
        let env = merge_env_vars(vec![
            ("CS_CLIENT_ACCESS_KEY", None),
            (
                "CS_WORKSPACE_CRN",
                Some("crn:us-west-1.aws:E4UMRN47WJNSMAKR"),
            ),
        ]);

        with_no_cs_vars(|| {
            temp_env::with_vars(env, || {
                let result = TandemConfig::build_path("tests/config/unknown.toml");
                assert!(result.is_err());

                if let Err(err) = result {
                    assert!(err
                        .to_string()
                        .contains("Missing client_access_key from [auth] configuration."));
                } else {
                    unreachable!();
                }
            })
        });
    }

    #[test]
    fn missing_encrypt_config() {
        // Missing all encrypt config
        let env = merge_env_vars(vec![
            ("CS_CLIENT_ID", None),
            ("CS_CLIENT_KEY", None),
            ("CS_DEFAULT_KEYSET_ID", None),
            (
                "CS_WORKSPACE_CRN",
                Some("crn:us-west-1.aws:E4UMRN47WJNSMAKR"),
            ),
        ]);

        with_no_cs_vars(|| {
            temp_env::with_vars(env, || {
                let result = TandemConfig::build_path("tests/config/unknown.toml");
                assert!(result.is_err());

                if let Err(err) = result {
                    assert!(err.to_string().contains("Missing [encrypt] configuration"));
                } else {
                    unreachable!();
                }
            })
        });

        // Missing client_id
        let env = merge_env_vars(vec![
            ("CS_CLIENT_ID", None),
            (
                "CS_WORKSPACE_CRN",
                Some("crn:us-west-1.aws:E4UMRN47WJNSMAKR"),
            ),
        ]);

        with_no_cs_vars(|| {
            temp_env::with_vars(env, || {
                let result = TandemConfig::build_path("tests/config/unknown.toml");
                assert!(result.is_err());

                if let Err(err) = result {
                    assert!(err
                        .to_string()
                        .contains("Missing client_id from [encrypt] configuration."));
                } else {
                    unreachable!();
                }
            })
        });
    }

    #[test]
    fn missing_database_config() {
        // Missing all database config

        let env = merge_env_vars(vec![
            ("CS_DATABASE__USERNAME", None),
            ("CS_DATABASE__PASSWORD", None),
            ("CS_DATABASE__NAME", None),
            ("CS_DATABASE__HOST", None),
            ("CS_DATABASE__PORT", None),
            (
                "CS_WORKSPACE_CRN",
                Some("crn:us-west-1.aws:E4UMRN47WJNSMAKR"),
            ),
        ]);

        with_no_cs_vars(|| {
            temp_env::with_vars(env, || {
                let result = TandemConfig::build_path("tests/config/unknown.toml");
                assert!(result.is_err());

                if let Err(err) = result {
                    assert!(err.to_string().contains("Missing [database] configuration"));
                } else {
                    unreachable!();
                }
            })
        });

        // Missing name
        let env = merge_env_vars(vec![
            ("CS_DATABASE__NAME", None),
            (
                "CS_WORKSPACE_CRN",
                Some("crn:us-west-1.aws:E4UMRN47WJNSMAKR"),
            ),
        ]);

        with_no_cs_vars(|| {
            temp_env::with_vars(env, || {
                let result = TandemConfig::build_path("tests/config/unknown.toml");
                assert!(result.is_err());

                if let Err(err) = result {
                    assert!(err
                        .to_string()
                        .contains("Missing name from [database] configuration."));
                } else {
                    unreachable!();
                }
            })
        });
    }

    #[test]
    fn crn_can_load_from_toml() {
        with_no_cs_vars(|| {
            let config =
                TandemConfig::build_path("tests/config/cipherstash-proxy-with-crn.toml").unwrap();
            assert_eq!(
                "E4UMRN47WJNSMAKR",
                config.auth.workspace_crn.workspace_id.to_string()
            );
            assert_eq!(
                "us-west-1.aws",
                config.auth.workspace_crn.region.to_string()
            );
        })
    }

    #[test]
    fn crn_can_load_from_env_vars() {
        let env = merge_env_vars(vec![(
            "CS_WORKSPACE_CRN",
            Some("crn:us-west-1.aws:E4UMRN47WJNSMAKR"),
        )]);

        with_no_cs_vars(|| {
            temp_env::with_vars(env, || {
                let config = TandemConfig::build_path("tests/config/unknown.toml").unwrap();
                assert_eq!(
                    "E4UMRN47WJNSMAKR",
                    config.auth.workspace_crn.workspace_id.to_string()
                );
                assert_eq!(
                    "us-west-1.aws",
                    config.auth.workspace_crn.region.to_string()
                );
            })
        })
    }

    #[test]
    fn slow_statement_accessors() {
        with_no_cs_vars(|| {
            temp_env::with_vars(
                [
                    ("CS_LOG__SLOW_STATEMENTS", Some("true")),
                    ("CS_LOG__SLOW_STATEMENT_MIN_DURATION_MS", Some("1000")),
                ],
                || {
                    let config =
                        TandemConfig::build_path("tests/config/cipherstash-proxy-test.toml")
                            .unwrap();

                    assert!(config.slow_statements_enabled());
                    assert_eq!(
                        config.slow_statement_min_duration(),
                        std::time::Duration::from_millis(1000)
                    );
                },
            );
        });
    }

    #[test]
    fn slow_statements_config() {
        with_no_cs_vars(|| {
            temp_env::with_vars(
                [
                    ("CS_LOG__SLOW_STATEMENTS", Some("true")),
                    ("CS_LOG__SLOW_STATEMENT_MIN_DURATION_MS", Some("500")),
                ],
                || {
                    let config =
                        TandemConfig::build_path("tests/config/cipherstash-proxy-test.toml")
                            .unwrap();

                    assert!(config.log.slow_statements);
                    assert_eq!(config.log.slow_statement_min_duration_ms, 500);
                },
            );
        });
    }

    #[test]
    fn thread_stack_size_honors_rust_min_stack() {
        temp_env::with_var("RUST_MIN_STACK", Some("8388608"), || {
            assert_eq!(
                TandemConfig::for_testing().thread_stack_size(),
                8 * 1024 * 1024
            );
        });
    }

    #[test]
    fn debug_builds_use_the_larger_thread_stack() {
        temp_env::with_var("RUST_MIN_STACK", None::<&str>, || {
            assert_eq!(
                TandemConfig::for_testing().thread_stack_size(),
                if cfg!(debug_assertions) {
                    crate::config::DEBUG_THREAD_STACK_SIZE
                } else {
                    crate::config::DEFAULT_THREAD_STACK_SIZE
                }
            );
        });
    }

    fn missing_tls_config() -> crate::config::TlsConfig {
        crate::config::TlsConfig::Path {
            certificate_path: "tests/tls/does-not-exist.cert".to_string(),
            private_key_path: "tests/tls/does-not-exist.key".to_string(),
        }
    }

    fn load_test_config(args: crate::Args) -> Result<TandemConfig, Error> {
        with_no_cs_vars(|| TandemConfig::load(&args))
    }

    fn test_args() -> crate::Args {
        use clap::Parser;
        crate::Args::parse_from([
            "cipherstash-proxy",
            "--config-file-path",
            "tests/config/cipherstash-proxy-test.toml",
        ])
    }

    #[test]
    /// An invalid certificate must not discard an explicit `require_tls`:
    /// falling back to plaintext would accept the sessions it exists to reject.
    fn invalid_tls_with_require_tls_is_fatal() {
        let mut config = TandemConfig::for_testing();
        config.tls = Some(missing_tls_config());
        config.server.require_tls = true;

        let result = config.resolve_inbound_tls();

        assert!(result.is_err(), "expected an error, got {result:?}");
    }

    #[test]
    /// TLS that was configured but is invalid is fatal: a plaintext fallback
    /// would silently downgrade clients that use `sslmode=prefer`, including
    /// after a SIGHUP reload.
    fn invalid_tls_without_require_tls_is_fatal() {
        let mut config = TandemConfig::for_testing();
        config.tls = Some(missing_tls_config());

        let result = config.resolve_inbound_tls();

        assert!(result.is_err(), "expected an error, got {result:?}");
    }

    #[test]
    /// `--tls` must require TLS from clients, not only a valid certificate.
    fn tls_flag_sets_require_tls() {
        let mut args = test_args();
        args.tls = true;

        let config = load_test_config(args).unwrap();

        assert!(config.server.require_tls);
    }

    #[test]
    fn tls_flag_without_tls_configuration_is_an_error() {
        let mut args = test_args();
        args.tls = true;
        args.config_file_path = "tests/config/does-not-exist.toml".to_string();

        let result = with_no_cs_vars(|| {
            temp_env::with_vars(
                [
                    ("CS_DATABASE__NAME", Some("test")),
                    ("CS_DATABASE__PASSWORD", Some("test")),
                    (
                        "CS_WORKSPACE_CRN",
                        Some("crn:ap-southeast-2.aws:IJGECSCWKREECNBS"),
                    ),
                    ("CS_CLIENT_ACCESS_KEY", Some("test")),
                    ("CS_CLIENT_ID", Some("00000000-0000-0000-0000-000000000001")),
                    (
                        "CS_CLIENT_KEY",
                        Some(TandemConfig::for_testing().encrypt.client_key.as_str()),
                    ),
                ],
                || TandemConfig::load(&args),
            )
        });

        assert!(
            matches!(
                result,
                Err(Error::Config(ConfigError::TlsConfigurationRequired))
            ),
            "expected TlsConfigurationRequired, got {result:?}"
        );
    }

    #[test]
    /// The quiet default filters `warn!`, so a plaintext listener needs a
    /// notice that is printed regardless of log level.
    fn tls_not_configured_prints_a_notice() {
        assert!(InboundTls::NotConfigured.notice(false).is_some());
    }

    #[test]
    fn no_tls_flag_silences_the_not_configured_notice() {
        assert_eq!(InboundTls::NotConfigured.notice(true), None);
    }

    /// Runs `f` with the CS_DATABASE__* vars that --database-url sets unset,
    /// and restores them afterwards.
    fn with_no_database_vars<F: FnOnce() -> R, R>(f: F) -> R {
        with_no_cs_vars(|| {
            temp_env::with_vars_unset(
                [
                    "CS_DATABASE__HOST",
                    "CS_DATABASE__PORT",
                    "CS_DATABASE__USERNAME",
                    "CS_DATABASE__PASSWORD",
                    "CS_DATABASE__NAME",
                ],
                f,
            )
        })
    }

    #[test]
    /// `sslmode` is not applied to the database connection, so accepting it
    /// would silently downgrade `sslmode=require` to `prefer`.
    fn database_url_rejects_sslmode() {
        use crate::config::tandem::apply_database_url;
        for sslmode in ["require", "disable"] {
            with_no_database_vars(|| {
                let url =
                    format!("postgres://alice:s3cret@db.example.com/orders?sslmode={sslmode}");
                let result = apply_database_url(&url);
                assert!(result.is_err(), "expected sslmode={sslmode} to be rejected");
            });
        }
    }

    #[test]
    fn database_url_accepts_sslmode_prefer() {
        use crate::config::tandem::apply_database_url;
        with_no_database_vars(|| {
            apply_database_url("postgres://alice:s3cret@db.example.com/orders?sslmode=prefer")
                .unwrap();
        });
    }

    #[test]
    /// A Unix socket host would leave CS_DATABASE__HOST unset while still
    /// setting the port from the same URL.
    fn database_url_rejects_unix_socket_host() {
        use crate::config::tandem::apply_database_url;
        with_no_database_vars(|| {
            let result =
                apply_database_url("postgres://alice@%2Fvar%2Frun%2Fpostgresql:5433/orders");
            assert!(result.is_err());
            assert!(std::env::var("CS_DATABASE__PORT").is_err());
        });
    }

    #[test]
    fn database_url_rejects_multiple_hosts() {
        use crate::config::tandem::apply_database_url;
        with_no_database_vars(|| {
            let result =
                apply_database_url("postgres://alice@db1.example.com,db2.example.com/orders");
            assert!(result.is_err());
            assert!(std::env::var("CS_DATABASE__HOST").is_err());
        });
    }

    #[test]
    /// Every connection option other than user, password, host, port and
    /// dbname would be silently ignored, so it is refused.
    fn database_url_rejects_unapplied_options() {
        use crate::config::tandem::apply_database_url;
        for option in [
            "channel_binding=require",
            "hostaddr=10.0.0.9",
            "port=5432,5433",
            "target_session_attrs=read-write",
            "options=-csearch_path%3Dfoo",
            "application_name=app",
            "connect_timeout=5",
            "load_balance_hosts=random",
            "sslnegotiation=direct",
        ] {
            with_no_database_vars(|| {
                let url = format!("postgres://alice@db.example.com/orders?{option}");
                let result = apply_database_url(&url);
                assert!(result.is_err(), "expected {option} to be rejected");
            });
        }
    }

    #[test]
    /// A URL without a port must not replace the configured port with 5432.
    fn database_url_without_port_keeps_configured_port() {
        use crate::config::tandem::apply_database_url;
        with_no_database_vars(|| {
            apply_database_url("postgres://alice@db.example.com/orders").unwrap();

            assert!(std::env::var("CS_DATABASE__PORT").is_err());
            assert_eq!(
                std::env::var("CS_DATABASE__HOST").unwrap(),
                "db.example.com"
            );
        });
    }

    #[test]
    fn database_url_with_port_sets_port() {
        use crate::config::tandem::apply_database_url;
        with_no_database_vars(|| {
            apply_database_url("postgres://alice@[::1]:6543/orders").unwrap();

            assert_eq!(std::env::var("CS_DATABASE__PORT").unwrap(), "6543");
        });
    }

    #[test]
    fn database_url_sslmode_errors_point_to_the_supported_setting() {
        use crate::config::tandem::apply_database_url;
        for option in [
            "sslmode=require",
            "sslmode=verify-full",
            "sslmode=verify-ca",
            "sslmode=allow",
            "sslrootcert=/etc/ssl/root.crt",
        ] {
            with_no_database_vars(|| {
                let url = format!("postgres://alice@db.example.com/orders?{option}");
                let err = apply_database_url(&url).unwrap_err().to_string();
                assert!(
                    err.contains("CS_DATABASE__WITH_TLS_VERIFICATION"),
                    "{option}: {err}"
                );
            });
        }

        with_no_database_vars(|| {
            let url = "postgres://alice@db.example.com/orders?sslmode=disable";
            let err = apply_database_url(url).unwrap_err().to_string();
            assert!(
                err.contains("CS_DEVELOPMENT__DISABLE_DATABASE_TLS"),
                "sslmode=disable: {err}"
            );
            assert!(!err.contains("CS_DATABASE__WITH_TLS_VERIFICATION"));
        });
    }
}
