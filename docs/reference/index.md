# CipherStash Proxy reference guide

This page contains reference documentation for configuring CipherStash Proxy and its features.

## Table of contents

- [Proxy config options](#proxy-config-options)
  - [Recommended settings for development](#recommended-settings-for-development)
  - [Docker-specific configuration](#docker-specific-configuration)
- [Prometheus metrics](#prometheus-metrics)
  - [Available metrics](#available-metrics)
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

## Multitenant

CipherStash Proxy supports multitenant applications by allowing clients to switch between different keysets at runtime. This enables a single Proxy instance to handle encrypted data for multiple tenants, with each tenant's data protected by separate encryption keys.

### Keyset Commands

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

### Usage Notes

- These commands must be executed before performing any encrypted operations
- The keyset remains active for the duration of the connection
- If a default keyset is configured in the Proxy, these commands cannot be used and will return an error
- Each tenant should use a separate database connection with their own keyset
- Keyset switching is connection-scoped and does not affect other connections



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
| `cipherstash_proxy_keyset_cipher_init_total`                           | Counter   | Number of times a new keyset-scoped cipher  has been initialized                     |
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

## Supported architectures

CipherStash Proxy is [available as a Docker container image](https://hub.docker.com/r/cipherstash/proxy) for `linux/arm64` architectures.

If you're interested in a Docker image for other architectures (like `linux/amd64`), upvote [this idea](https://github.com/cipherstash/proxy/discussions/214).



---

### Didn't find what you wanted?

[Click here to let us know what was missing from our docs.](https://github.com/cipherstash/proxy/issues/new?template=docs-feedback.yml&title=[Docs:]%20Feedback%20on%20reference.md)
