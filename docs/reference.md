# CipherStash Proxy reference guide

This page contains reference documentation for configuring CipherStash Proxy and its features.

## Table of contents

- [Proxy config options](#proxy-config-options)
  - [Recommended settings for development](#recommended-settings-for-development)
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

# Enforce TLS connections
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

# Enable TLS verification
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


[tls]
# Certificate path
# Env: CS_TLS__CERTIFICATE_PATH
certificate_path = "./server.cert"

# Private Key path
# Env: CS_TLS__PRIVATE_KEY_PATH
private_key_path = "./server.key"

# Certificate path
# Env: CS_TLS__CERTIFICATE_PEM
certificate_pem = "..."

# Private Key path
# Env: CS_TLS__PRIVATE_KEY_PEM
private_key_pem = "..."


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
CS_LOG__FORMAT = "pretty"
CS_LOG__ANSI_ENABLED = "true"
```

If you are frequently changing the database schema or making updates to the column encryption configuration, it can be useful to reload the config and schema more frequently:

```bash
CS_DATABASE__CONFIG_RELOAD_INTERVAL = "10"
CS_DATABASE__SCHEMA_RELOAD_INTERVAL = "10"
```

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

| Name                                                  | Target    | Description                                                                 |
|-------------------------------------------------------|-----------|-----------------------------------------------------------------------------|
| `cipherstash_proxy_clients_active_connections`        | Gauge     | Current number of connections to CipherStash Proxy from clients             |
| `cipherstash_proxy_clients_bytes_received_total`      | Counter   | Number of bytes received by CipherStash Proxy from clients                  |
| `cipherstash_proxy_clients_bytes_sent_total`          | Counter   | Number of bytes sent from CipherStash Proxy to clients                      |
| `cipherstash_proxy_decrypted_values_total`            | Counter   | Number of individual values that have been decrypted                        |
| `cipherstash_proxy_decryption_duration_seconds`       | Histogram | Duration of time CipherStash Proxy spent performing decryption operations   |
| `cipherstash_proxy_decryption_duration_seconds_count` | Counter   | Number of observations of requests to CipherStash ZeroKMS to decrypt values |
| `cipherstash_proxy_decryption_duration_seconds_sum`   | Counter   | Total time CipherStash Proxy spent performing decryption operations         |
| `cipherstash_proxy_decryption_error_total`            | Counter   | Number of decryption operations that were unsuccessful                      |
| `cipherstash_proxy_decryption_requests_total`         | Counter   | Number of requests to CipherStash ZeroKMS to decrypt values                 |
| `cipherstash_proxy_encrypted_values_total`            | Counter   | Number of individual values that have been encrypted                        |
| `cipherstash_proxy_encryption_duration_seconds`       | Histogram | Duration of time CipherStash Proxy spent performing encryption operations   |
| `cipherstash_proxy_encryption_duration_seconds_count` | Counter   | Number of observations of requests to CipherStash ZeroKMS to encrypt values |
| `cipherstash_proxy_encryption_duration_seconds_sum`   | Counter   | Total time CipherStash Proxy spent performing encryption operations         |
| `cipherstash_proxy_encryption_error_total`            | Counter   | Number of encryption operations that were unsuccessful                      |
| `cipherstash_proxy_encryption_requests_total`         | Counter   | Number of requests to CipherStash ZeroKMS to encrypt values                 |
| `cipherstash_proxy_rows_encrypted_total`              | Counter   | Number of encrypted rows returned to clients                                |
| `cipherstash_proxy_rows_passthrough_total`            | Counter   | Number of non-encrypted rows returned to clients                            |
| `cipherstash_proxy_rows_total`                        | Counter   | Total number of rows returned                                               |
| `cipherstash_proxy_server_bytes_received_total`       | Counter   | Number of bytes CipherStash Proxy received from the PostgreSQL server       |
| `cipherstash_proxy_server_bytes_sent_total`           | Counter   | Number of bytes CipherStash Proxy sent to the PostgreSQL server             |
| `cipherstash_proxy_statements_duration_seconds`       | Histogram | Duration of time CipherStash Proxy spent executing SQL statements           |
| `cipherstash_proxy_statements_duration_seconds_count` | Count     | Number of observations of CipherStash Proxy statement duration              |
| `cipherstash_proxy_statements_duration_seconds_sum`   | Count     | Total time CipherStash Proxy spent executing SQL statements                 |
| `cipherstash_proxy_statements_encrypted_total`        | Counter   | Number of SQL statements that required encryption                           |
| `cipherstash_proxy_statements_passthrough_total`      | Counter   | Number of SQL statements that did not require encryption                    |
| `cipherstash_proxy_statements_total`                  | Counter   | Total number of SQL statements processed by CipherStash Proxy               |
| `cipherstash_proxy_statements_unmappable_total`       | Counter   | Total number of unmappable SQL statements processed by CipherStash Proxy    |

## Supported architectures

CipherStash Proxy is [available as a Docker container image](https://hub.docker.com/r/cipherstash/proxy) for `linux/arm64` architectures.

If you're interested in a Docker image for other architectures (like `linux/amd64`), upvote [this idea](https://github.com/cipherstash/proxy/discussions/214).



---

### Didn't find what you wanted?

[Click here to let us know what was missing from our docs.](https://github.com/cipherstash/proxy/issues/new?template=docs-feedback.yml&title=[Docs:]%20Feedback%20on%20reference.md)
