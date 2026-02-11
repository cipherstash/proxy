# CipherStash Proxy reference guide

This page contains reference documentation for configuring CipherStash Proxy and its features.

## Table of contents

- [Proxy config options](#proxy-config-options)
  - [Recommended settings for development](#recommended-settings-for-development)
  - [Docker-specific configuration](#docker-specific-configuration)
- [Prometheus metrics](#prometheus-metrics)
  - [Available metrics](#available-metrics)
- [Troubleshooting ZeroKMS connections](#troubleshooting-zerokms-connections)
- [Supported architectures](#supported-architectures)


## Proxy config options

You can configure CipherStash Proxy with a config file, enviroment variables, or a combination of the two – see [Configuring Proxy](#configuring-proxy) for instructions.

The following are all the configuration options available for Proxy, with their equivalent environment variables:

```toml

[server]
# Proxy host address
# Optional
# Default: `0.0.0.0`
# Env: CS_SERVER__HOST
host = "0.0.0.0"

# Proxy host posgt
# Optional
# Default: `6432`
# Env: CS_SERVER__PORT
port = "6432"

# Enforce TLS connections from the Client to Proxy
# Optional
# Default: `false`
# Env: CS_SERVER__REQUIRE_TLS
require_tls = "false"

# Shutdown timeout in ms
# Sets how long to wait for connections to drain on shutdown
# Optional
# Default: `2000`
# Env: CS_SERVER__SHUTDOWN_TIMEOUT
shutdown_timeout = "2000"

# Number of worker threads the server should use
# Optional
# Default: `NUMBER_OF_CORES/2` or `4`
# Env: CS_SERVER__WORKER_THREADS
worker_threads = "4"

# Thread stack size in bytes
# Optional
# Default: `2 * 1024 * 1024` (2MiB) or `4 * 1024 * 1024` (4MiB) if log level is DEBUG or TRACE
# Env: CS_SERVER__THREAD_STACK_SIZE
thread_stack_size = "2097152"

# Cipher cache size (number of entries)
# Sets the maximum number of encryption/decryption operations to cache
# Optional
# Default: `64`
# Env: CS_SERVER__CIPHER_CACHE_SIZE
cipher_cache_size = "64"

# Cipher cache TTL in seconds
# Sets how long cached encryption/decryption operations are valid
# Optional
# Default: `3600` (1 hour)
# Env: CS_SERVER__CIPHER_CACHE_TTL_SECONDS
cipher_cache_ttl_seconds = "3600"

### Proxy -> Backing database connection settings
[database]
# Database host address
# Optional
# Default: `0.0.0.0`
# Env: CS_DATABASE__HOST
host = "0.0.0.0"

# Database host post
# Optional
# Default: `5432`
# Env: CS_DATABASE__PORT
port = "5432"

# Database name
# Env: CS_DATABASE__NAME
name = "database"

# Database username
# Env: CS_DATABASE__USERNAME
username = "username"

# Database password
# Env: CS_DATABASE__PASSWORD
password = "password"

# Connection timeout in ms
# Sets how long to hold an open idle connection
# In production environments this should be greater than the idle timeout of any connection pool in the application.
#
# Optional
# No Default (NO TIMEOUT)
# Env: CS_DATABASE__CONNECTION_TIMEOUT
connection_timeout = "300000"

# Enable TLS verification between Proxy and the backing database.
# Optional
# Default: `false`
# Env: CS_DATABASE__WITH_TLS_VERIFICATION
with_tls_verification = "false"

# Encrypt configuration reload interval in sec
# Sets how frequently Encrypted index configuration should be reloaded
# The configuration specifies the encrypted columns in the database
# Optional
# Default: `60`
# Env: CS_DATABASE__CONFIG_RELOAD_INTERVAL
config_reload_interval = "60"

# Schema configuration reload interval in sec
# Sets how frequently the database schema should be reloaded
# The schema is used to analyse SQL statements
# Optional
# Default: `60`
# Env: CS_DATABASE__SCHEMA_RELOAD_INTERVAL
schema_reload_interval = "60"


### Client->Proxy TLS Settings:
# This section configures how the Proxy accepts connections from your client.
# A Public Certificate and Private Key pair is required to correctly enable TLS.
# Fill out:
# - a Certificate Path and a Private Key Path, *or*
# - a Certificate PEM string and a Private Key PEM string, *or*
# - neither, removing this section, to disable Client->Proxy TLS.
#   (This is a misconfiguration if `require_tls` above is enabled.)
[tls]
# Path to the Public Certificate .crt file.
# Env: CS_TLS__CERTIFICATE_PATH
certificate_path = "./server.crt"

# Path to the Private Key file.
# Env: CS_TLS__PRIVATE_KEY_PATH
private_key_path = "./server.key"

# The Public Certificate PEM as a string.
# Env: CS_TLS__CERTIFICATE_PEM
certificate_pem = "-----BEGIN CERTIFICATE----- ... -----END CERTIFICATE-----"

# The Private Key as a string.
# Env: CS_TLS__PRIVATE_KEY_PEM
private_key_pem = "-----BEGIN RSA PRIVATE KEY----- ... -----END RSA PRIVATE KEY-----"


[auth]
# CipherStash Workspace CRN
# Env: CS_WORKSPACE_CRN
workspace_crn = "cipherstash-workspace-crn"

# CipherStash Client Access Key
# Env: CS_CLIENT_ACCESS_KEY
client_access_key = "cipherstash-client-access-key"

[encrypt]
# CipherStash Dataset ID
# Env: CS_DEFAULT_KEYSET_ID
default_keyset_id = "cipherstash-dataset-id"

# CipherStash Client ID
# Env: CS_CLIENT_ID
client_id = "cipherstash-client-id"

# CipherStash Client Key
# Env: CS_CLIENT_KEY
client_key = "cipherstash-client-key"


[log]
# Log level
# Optional
# Valid values: `error | warn | info | debug | trace`
# Default: `info`
# Env: CS_LOG__LEVEL
level = "info"

# Log format
# Optional
# Valid values: `pretty | text | structured (json)`
# Default: `pretty` if Proxy detects during startup that a terminal is attached, otherwise `structured`
# Env: CS_LOG__FORMAT
format = "pretty"

# Log format
# Optional
# Valid values: `stdout | stderr`
# Default: `info`
# Env: CS_LOG__OUTPUT
output = "stdout"

# Enable ansi (colored) output
# Optional
# Default: `true` if Proxy detects during startup that a terminal is attached, otherwise `false`
# Env: CS_LOG__ANSI_ENABLED
ansi_enabled = "true"


[prometheus]
# Enable prometheus stats
# Optional
# Default: `false`
# Env: CS_PROMETHEUS__ENABLED
enabled = "false"

# Prometheus exporter post
# Optional
# Default: `9930`
# Env: CS_PROMETHEUS__PORT
port = "9930"
```

### Recommended settings for development

The default configuration settings are biased toward running in production environments.
Although Proxy attempts to detect the environment and set a sensible default for logging, your mileage may vary.

To turn on human-friendly logging:

```bash
CS_LOG__FORMAT="pretty"
CS_LOG__ANSI_ENABLED="true"
```

If you are frequently changing the database schema or making updates to the column encryption configuration, it can be useful to reload the config and schema more frequently:

```bash
CS_DATABASE__CONFIG_RELOAD_INTERVAL="10"
CS_DATABASE__SCHEMA_RELOAD_INTERVAL="10"
```

### Docker-specific configuration

As a convenience for local development, if you use [Proxy's Docker container](../proxy.Dockerfile) with its default entrypoint and the below environment variables set, the EQL SQL will be applied to the database, and an example schema (for example, with the `users` table, from the [README Getting Started example](../README.md#getting-started)) will be loaded. These are turned on by default in the [development `docker-compose.yml`](../docker-compose.yml):

(It is not recommended to use either of these in production.)

```bash
CS_DATABASE__INSTALL_EQL="true"
CS_DATABASE__INSTALL_EXAMPLE_SCHEMA="true"
```

As a convenience for production deployments, with the below environment variable set, the Proxy container will add the AWS RDS global certificate bundle to the operating system's set of trusted certificates. This is recommended when running Proxy on AWS.

```bash
CS_DATABASE__INSTALL_AWS_RDS_CERT_BUNDLE="true"
```

## Command line options



## Command line interface

The CipherStash Proxy accepts command line arguments.
For example, the upstream database can be specified via command line arguments.
Useful for local dev and testing.

### Usage

```bash
cipherstash-proxy [OPTIONS] [DBNAME] [COMMAND]
```

### Commands

- **encrypt**  
  Encrypt one or more columns in a table. This command requires a running and properly configured CipherStash Proxy instance.

- **help**  
  Print the help message or detailed information for the specified subcommand(s).

### Arguments

- **DBNAME**  

  Optional name of the database to connect to. If not specified, the tool will use the environment variables or configuration file settings.

  Default value: none

- **-H, --db-host <DB_HOST>**

  Optional database host. This value will default to the one defined in your environment or configuration file if not provided.

  Default value: `127.0.0.1`

- **-u, --db-user <DB_USER>**

  Optional database user. This value will default to the one defined in your environment or configuration file if not provided.

  Default value: `postgres`

- **-p, --config-file-path <CONFIG_FILE_PATH>**

  Specifies an optional path to a CipherStash Proxy configuration file.
  If provided, the application attempts to load configuration settings from this file.
  However, environment variables can be used instead of the file or to override any values defined within it.

  Default Value: `cipherstash-proxy.toml`
    
  Note:
  The application will look for "cipherstash-proxy.toml" by default if no other file path is specified.
  
- **-l, --log-level <LOG_LEVEL>**
  
  Sets an optional log level for the application, which controls the verbosity of the logging output.
  This can be particularly useful for adjusting the level of detail in application logs
  to suit different environments or debugging needs.
    
  Default Value: `info`
    
  Environment Variable: `CS_LOG__LEVEL`
    
  Possible Values: `error`, `warn`, `info`, `debug`, `trace`
  
- **-f, --log-format <LOG_FORMAT>**
    
  Specifies an optional log format for the output logs.
  The default log format is "pretty" when the application detects that it is running in a terminal session,
  otherwise it defaults to "structured" for non-interactive environments.
  The setting can be overridden by the corresponding environment variable.
    
  Default Value: `pretty` (if running in a terminal session), otherwise `structured`
    
  Environment Variable: `CS_LOG__FORMAT`
    
  Possible Values: `pretty`, `structured`, `text`
  

## Multitenant operation

CipherStash Proxy supports multitenant applications using ZeroKMS keysets to provide strong cryptographic separation between tenants.

In multitenant operation, tenants are associated with a keyset, and data is protected by separate encryption keys. Data access through the proxy can be scoped to a specific keyset at runtime using the `SET CIPHERSTASH.KEYSET` SQL commands:
  - `SET CIPHERSTASH.KEYSET_ID`
  - `SET CIPHERSTASH.KEYSET_NAME`

The `SET CIPHERSTASH.KEYSET` commands enable a proxy connection to be scoped to a keyset by `id` or `name`. Once a keyset has been set for a connection, subsequent operations are scoped to that keyset. Data can only be decrypted by the same keyset that performed the encryption.

 A keyset `name` is unique to a workspace, and functions like an alias. Using a keyset `name` enables the keyset to be associated with an arbitrary identifier such as an internal `TenantId`. Use of a `name` is optional, and the actual `id`

The proxy must be configured *without* a `DEFAULT_KEYSET_ID` to enable multitenant operation and the use of the `SET KEYSET` commands.


### Keyset commands

#### SET CIPHERSTASH.KEYSET_ID

Sets the active keyset for the current connection using a keyset UUID.

**Syntax:**
```sql
SET CIPHERSTASH.KEYSET_ID = '<keyset-uuid>';
```

**Parameters:**
- `keyset-uuid`: The UUID of the keyset to activate for this connection

**Example:**
```sql
SET CIPHERSTASH.KEYSET_ID = '2cace9db-3a2a-4b46-a184-ba412b3e0730';
```

#### SET CIPHERSTASH.KEYSET_NAME

Sets the active keyset for the current connection using a keyset name.

**Syntax:**
```sql
SET CIPHERSTASH.KEYSET_NAME = '<keyset-name>';
```

**Parameters:**
- `keyset-name`: The name of the keyset to activate for this connection

**Example:**
```sql
SET CIPHERSTASH.KEYSET_NAME = 'tenant-1';
```

### Usage notes

- The `SET CIPHERSTASH.KEYSET` commands must be executed before performing any encryption operations
- The keyset remains active for the duration of the connection, or until a subsequent `SET CIPHERSTASH.KEYSET`
- If a default keyset is configured in the Proxy, these commands cannot be used, and will return an error
- The active keyset is connection-scoped and does not affect other connections


## Disabling encrypted mapping
Transforming SQL statements is core to how CipherStash Proxy works.
Internally, Proxy takes the plaintext SQL statements issued by your application, and transforms them into statements on [EQL](https://github.com/cipherstash/encrypt-query-language/) columns.
Proxy does this through an internal process called _encrypted mapping_, or just _mapping_ for short.

In some circumstances it may be necessary to disable encrypted mapping for one or more SQL statements.

For example, you are doing a data transformation with complex logic, and you are doing the transformation directly in the database with `plpgsql`.

A `SET` command can be used to change the `CIPHERSTASH.UNSAFE_DISABLE_MAPPING` configuration parameter.

The parameter is always scoped to the connection `SESSION` - mapping is only ever disabled for the client connection the `SET` command was issued on.

> [!IMPORTANT]
> Extra care is required when using `CIPHERSTASH.UNSAFE_DISABLE_MAPPING`.
>
> **If mapping is disabled, sensitive data may not be encrypted and may appear in logs.**

CipherStash Proxy and EQL do provide some protection against writing plaintext into and reading plaintext from encrypted columns.

Always use `eql_v2.add_encrypted_constraint(table, column)` when defining encrypted columns to ensure plaintext data cannot be written.

Unmapped `SELECT` statements should always return the encrypted payload.
If the constraint has been applied, unmapped `INSERT`/`UPDATE` statements should return a PostgreSQL type error.


### Disable mapping
```
SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true;
```

### Enable mapping
```
SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = false;
```

### Note on prepared statements and mapping

CipherStash Proxy only decrypts data of SQL statements that it has explicitly checked and mapped.

If mapping is disabled, any subsequent `PREPARE` will skip the mapping process.

If mapping is re-enabled for the connection, returned data will not be decrypted.

To enable mapping, encryption, and decryption of prepared statements, either:

- a new connection is required, or
- the client needs to prepare the statement again

This behaviour is expected and a consequence of the PostgreSQL protocol.

To prepare a statement, the client sends the SQL in a `parse` message.
Once a statement has been prepared, the client skips the `parse` step, and does not send the SQL again, referring to the statement by a specified name.
If mapping is disabled, Proxy will not map the statement on `parse`, and data returned from subsequent executions will never be decrypted.
If mapping is enabled on the connection, when the client executes the statement it will reference it by name, skipping the `parse` step.
As the statement was not mapped in the `parse` because mapping was disabled at that point, the returned data will not be decrypted


## Prometheus metrics

To enable a Prometheus exporter on the default port (`9930`) use either:

```toml
[prometheus]
enabled = "true"
```

```env
CS_PROMETHEUS__ENABLED = "true"
```

When enabled, metrics can be accessed via `http://localhost:9930/metrics`.
If the proxy is running on a host other than localhost, access on that host.


### Available metrics

| Name                                                            | Target    | Description                                                                 |
|-----------------------------------------------------------------|-----------|-----------------------------------------------------------------------------|
| `cipherstash_proxy_keyset_cipher_cache_hits_total`                     | Counter   | Number of times a keyset-scoped cipher was found in the cache                       |
| `cipherstash_proxy_keyset_cipher_cache_miss_total`                     | Counter   | Number of cipher cache misses requiring initialization                               |
| `cipherstash_proxy_keyset_cipher_init_total`                           | Counter   | Number of times a new keyset-scoped cipher  has been initialized                     |
| `cipherstash_proxy_keyset_cipher_init_duration_seconds`                | Histogram | Duration of cipher initialization including ZeroKMS network call                     |
| `cipherstash_proxy_clients_active_connections`                  | Gauge     | Current number of connections to CipherStash Proxy from clients             |
| `cipherstash_proxy_clients_bytes_received_total`                | Counter   | Number of bytes received by CipherStash Proxy from clients                  |
| `cipherstash_proxy_clients_bytes_sent_total`                    | Counter   | Number of bytes sent from CipherStash Proxy to clients                      |
| `cipherstash_proxy_decrypted_values_total`                      | Counter   | Number of individual values that have been decrypted                        |
| `cipherstash_proxy_decryption_duration_seconds`                 | Histogram | Duration of time CipherStash Proxy spent performing decryption operations   |
| `cipherstash_proxy_decryption_duration_seconds_count`           | Counter   | Number of observations of requests to CipherStash ZeroKMS to decrypt values |
| `cipherstash_proxy_decryption_duration_seconds_sum`             | Counter   | Total time CipherStash Proxy spent performing decryption operations         |
| `cipherstash_proxy_decryption_error_total`                      | Counter   | Number of decryption operations that were unsuccessful                      |
| `cipherstash_proxy_decryption_requests_total`                   | Counter   | Number of requests to CipherStash ZeroKMS to decrypt values                 |
| `cipherstash_proxy_encrypted_values_total`                      | Counter   | Number of individual values that have been encrypted                        |
| `cipherstash_proxy_encryption_duration_seconds`                 | Histogram | Duration of time CipherStash Proxy spent performing encryption operations   |
| `cipherstash_proxy_encryption_duration_seconds_count`           | Counter   | Number of observations of requests to CipherStash ZeroKMS to encrypt values |
| `cipherstash_proxy_encryption_duration_seconds_sum`             | Counter   | Total time CipherStash Proxy spent performing encryption operations         |
| `cipherstash_proxy_encryption_error_total`                      | Counter   | Number of encryption operations that were unsuccessful                      |
| `cipherstash_proxy_encryption_requests_total`                   | Counter   | Number of requests to CipherStash ZeroKMS to encrypt values                 |
| `cipherstash_proxy_rows_encrypted_total`                        | Counter   | Number of encrypted rows returned to clients                                |
| `cipherstash_proxy_rows_passthrough_total`                      | Counter   | Number of non-encrypted rows returned to clients                            |
| `cipherstash_proxy_rows_total`                                  | Counter   | Total number of rows returned                                               |
| `cipherstash_proxy_server_bytes_received_total`                 | Counter   | Number of bytes CipherStash Proxy received from the PostgreSQL server       |
| `cipherstash_proxy_server_bytes_sent_total`                     | Counter   | Number of bytes CipherStash Proxy sent to the PostgreSQL server             |
| `cipherstash_proxy_statements_execution_duration_seconds`       | Histogram | Duration of time the proxied database spent executing SQL statements        |
| `cipherstash_proxy_statements_execution_duration_seconds_count` | Count     | Number of observations of CipherStash Proxy statement duration              |
| `cipherstash_proxy_statements_execution_duration_seconds_sum`   | Count     | Total time CipherStash Proxy spent executing SQL statements                 |
| `cipherstash_proxy_statements_session_duration_seconds`         | Histogram | Duration of time CipherStash Proxy spent processing the statement including encryption, proxied database execution, and decryption |
| `cipherstash_proxy_statements_session_duration_seconds_count`   | Count     | Number of observations of CipherStash Proxy statement duration              |
| `cipherstash_proxy_statements_session_duration_seconds_sum`     | Count     | Total time CipherStash Proxy spent processing SQL statements                |
| `cipherstash_proxy_statements_encrypted_total`                  | Counter   | Number of SQL statements that required encryption                           |
| `cipherstash_proxy_statements_passthrough_total`                | Counter   | Number of SQL statements that did not require encryption                    |
| `cipherstash_proxy_statements_total`                            | Counter   | Total number of SQL statements processed by CipherStash Proxy               |
| `cipherstash_proxy_statements_unmappable_total`                 | Counter   | Total number of unmappable SQL statements processed by CipherStash Proxy    |

## Troubleshooting ZeroKMS connections

### Recommended log settings

For a quick check on ZeroKMS latency and connection issues:

```bash
CS_LOG__ZERO_KMS_LEVEL=debug
CS_LOG__ENCRYPT_LEVEL=debug
```

For a deeper investigation, also enable slow statement detection:

```bash
CS_LOG__ZERO_KMS_LEVEL=trace
CS_LOG__SLOW_STATEMENTS=true
CS_LOG__SLOW_STATEMENT_MIN_DURATION_MS=500
CS_LOG__SLOW_DB_RESPONSE_MIN_DURATION_MS=50
```

### What to look for in logs

| Signal | Meaning |
|--------|---------|
| `Initializing ZeroKMS ScopedCipher (cache miss)` | Network call to ZeroKMS is about to happen. Frequent occurrences indicate cache churn. |
| `Connected to ZeroKMS` with high `init_duration_ms` | Slow cipher init. Healthy values are <200ms; >1s triggers a warning. |
| `Use cached ScopedCipher` | Cache hit (fast path, no network call). |
| `ScopedCipher evicted from cache` with `cause: Size` | Cache too small for workload. Increase `cipher_cache_size`. |
| `Error initializing ZeroKMS` with high `init_duration_ms` | Network timeout to ZeroKMS. |
| `Error initializing ZeroKMS` with low `init_duration_ms` | Credential or configuration error. |

### Key metrics

Enable Prometheus (`CS_PROMETHEUS__ENABLED=true`) and watch these metrics:

| Metric | Why |
|--------|-----|
| `cipherstash_proxy_keyset_cipher_init_duration_seconds` | Distribution of ZeroKMS init times including network latency. |
| `cipherstash_proxy_keyset_cipher_cache_hits_total` / `cache_miss_total` | Cache hit ratio — should be >95% in steady state. |
| `cipherstash_proxy_statements_session_duration_seconds` minus `execution_duration_seconds` | Encryption overhead per statement (large gap = encryption, not database). |
| `cipherstash_proxy_encryption_error_total` / `decryption_error_total` | Spikes indicate ZeroKMS connectivity issues. |

### Useful PromQL queries

```promql
# P99 cipher init latency
histogram_quantile(0.99, rate(cipherstash_proxy_keyset_cipher_init_duration_seconds_bucket[5m]))

# Cache hit ratio
rate(cipherstash_proxy_keyset_cipher_cache_hits_total[5m])
/ (rate(cipherstash_proxy_keyset_cipher_cache_hits_total[5m])
   + rate(cipherstash_proxy_keyset_cipher_cache_miss_total[5m]))
```

### Quick checklist

1. **Cache hit ratio low?** Tune `CS_SERVER__CIPHER_CACHE_SIZE` (default: 64) and `CS_SERVER__CIPHER_CACHE_TTL_SECONDS` (default: 3600).
2. **`init_duration_ms` >1s?** Network latency to ZeroKMS. Check DNS, firewall rules, and regional proximity.
3. **Large session vs execution duration gap?** Overhead is in encrypt/decrypt, not the database.
4. **Frequent evictions?** Increase `cipher_cache_size` to match your workload's keyset count.


## Supported architectures

CipherStash Proxy is [available as a Docker container image](https://hub.docker.com/r/cipherstash/proxy) for `linux/arm64` architectures.

If you're interested in a Docker image for other architectures (like `linux/amd64`), upvote [this idea](https://github.com/cipherstash/proxy/discussions/214).



---

### Didn't find what you wanted?

[Click here to let us know what was missing from our docs.](https://github.com/cipherstash/proxy/issues/new?template=docs-feedback.yml&title=[Docs:]%20Feedback%20on%20reference.md)
