<h1 align="center">
  <img alt="CipherStash Logo" loading="lazy" width="128" height="128" decoding="async" data-nimg="1"   style="color:transparent" src="https://cipherstash.com/assets/cs-github.png">
  </br>
  Proxy
</h1>
<p align="center">
  Implement robust data security without sacrificing performance or usability
  <br/>
  <div align="center" style="display: flex; justify-content: center; gap: 1rem;">
    <a href="https://cipherstash.com">
      <img
        src="https://raw.githubusercontent.com/cipherstash/meta/refs/heads/main/csbadge.svg"
        alt="Built by CipherStash"
      />
    </a>
    <a href="https://hub.docker.com/r/cipherstash/proxy">
      <img
        alt="Docker Pulls"
        src="https://img.shields.io/docker/pulls/cipherstash/proxy?style=for-the-badge&labelColor=000000"
      />
    </a>
  </div>
</p>
<br/>

<!-- start -->

# What's CipherStash Proxy?

CipherStash Proxy provides a transparent proxy to your existing postgres database, handling the complexity of encrypting and decrypting your data.
CipherStash Proxy keeps your sensitive data in PostgreSQL encrypted and searchable, without changing your SQL queries.

Behind the scenes, it uses the [Encrypt Query Language](https://github.com/cipherstash/encrypt-query-language/) to index and search encrypted data.

## Table of contents

- [Getting started](#getting-started)
- [How-to](#how-to)
  - [Installing Proxy](#installing-proxy)
  - [Configuring Proxy](#configuring-proxy)
    - [Configuring Proxy with environment variables](#configuring-proxy-with-environment-variables)
    - [Configuring Proxy with a TOML file](#configuring-proxy-with-a-toml-file)
  - [Running Proxy locally](#running-proxy-locally)
  - [Setting up the database schema](#setting-up-the-database-schema)
    - [Creating columns with the right types](#creating-columns-with-the-right-types)
- [Reference](#reference)
  - [Proxy config options](#proxy-config-options)
  - [Prometheus metrics](#prometheus-metrics)
    - [Available metrics](#available-metrics)
- [More info](#more-info)
  - [Developing for Proxy](#developing-for-proxy)

## Getting started

> [!IMPORTANT]
> **Prerequisites:** Before you start you need to have this software installed:
>  - [Docker](https://www.docker.com/) — see Docker's [documentation for installing](https://docs.docker.com/get-started/get-docker/)

Get up and running in local dev in < 5 minutes:

```bash
# Clone the repo
git clone https://github.com/cipherstash/proxy
cd proxy

# Setup your CipherStash configuration
stash setup
# TODO: get the config into a format cipherstash-proxy can read

# Start the containers
docker compose up
# TODO: Install a database schema: https://docs.docker.com/compose/how-tos/lifecycle/

# TODO: Run a query
psql postgres://${CS_DATABASE__USERNAME}:${CS_DATABASE__PASSWORD}@localhost:6432/cipherstash

# TODO: Verify the data is encrypted
psql postgres://${CS_DATABASE__USERNAME}:${CS_DATABASE__PASSWORD}@postgres:5432/cipherstash
```

## How-to

This section contains how-to documentation for installing, configuring, and running CipherStash Proxy.

### Installing Proxy

CipherStash Proxy is available as a [container image](https://hub.docker.com/r/cipherstash/proxy) on Docker Hub that can be deployed locally, in CI/CD, through to production.

The easiest way to start using CipherStash Proxy with your application is by adding a container to your application's `docker-compose.yml`.
The following is an example of what adding CipherStash Proxy to your app's `docker-compose.yml` might look like:

```yaml
services:
  app:
    # Your Postgres container config
  db:
    # Your Postgres container config
  proxy:
    image: cipherstash/proxy:latest
    container_name: proxy
    ports:
      - 6432:6432
      - 9930:9930
    environment:
      # Hostname of the Postgres database server connections will be proxied to
      - CS_DATABASE__HOST=${CS_DATABASE__HOST}
      # Port of the Postgres database server connections will be proxied to
      - CS_DATABASE__PORT=${CS_DATABASE__PORT}
      # Username of the Postgres database server connections will be proxied to
      - CS_DATABASE__USERNAME=${CS_DATABASE__USERNAME}
      # Password of the Postgres database server connections will be proxied to
      - CS_DATABASE__PASSWORD=${CS_DATABASE__PASSWORD}
      # The database name on the Postgres database server connections will be proxied to
      - CS_DATABASE__NAME=${CS_DATABASE__NAME}
      # The CipherStash workspace ID for making requests for encryption keys
      - CS_AUTH__WORKSPACE_ID=${CS_AUTH__WORKSPACE_ID}
      # The CipherStash client access key for making requests for encryption keys
      - CS_AUTH__CLIENT_ACCESS_KEY=${CS_AUTH__CLIENT_ACCESS_KEY}
      # The CipherStash dataset ID for generating and retrieving encryption keys
      - CS_ENCRYPT__DATASET_ID=${CS_ENCRYPT__DATASET_ID}
      # The CipherStash client ID used to programmatically access a dataset
      - CS_ENCRYPT__CLIENT_ID=${CS_ENCRYPT__CLIENT_ID}
      # The CipherStash client key used to programmatically access a dataset
      - CS_ENCRYPT__CLIENT_KEY=${CS_ENCRYPT__CLIENT_KEY}
      # Toggle Prometheus exporter for CipherStash Proxy operations
      - CS_PROMETHEUS__ENABLED=${CS_PROMETHEUS__ENABLED:-true}
```

For a fully-working example, go to [`docker-compose.yml`](./docker-compose.yml). Follow the steps in [Getting started](#getting-started) to see it in action.

Once you have set up a `docker-compose.yml`, start the Proxy container:

```bash
docker compose up
```

Connect your PostgreSQL client to Proxy on TCP 6432.
Point [Prometheus to scrape metrics](#prometheus-metrics) on TCP 9930.

### Configuring Proxy

To run, CipherStash Proxy needs to know:

- What port to run on
- How to connect to the target PostgreSQL database
- Secrets to authenticate to CipherStash

There are two ways to configure Proxy:

- [With environment variables that Proxy looks up on startup](#configuring-proxy-with-environment-variables)
- [With a TOML file that Proxy reads on startup](#configuring-proxy-with-a-toml-file)

Proxy's configuration loading order of preference is:

1. If `cipherstash-proxy.toml` is present in the current working directory, Proxy will read its config from that file
1. If `cipherstash-proxy.toml` is not present, Proxy will look up environment variables to configure itself
1. If **both** `cipherstash-proxy.toml` and environment variables are present, Proxy will use `cipherstash-proxy.toml` as the base configuration, and override it with any environment variables that are set

See [Proxy config options](#proxy-config-options) for all the available options.

#### Configuring Proxy with environment variables

If you are configuring Proxy with environment variables, these are the minimum environment variables required to run Proxy:

```bash
CS_DATABASE__NAME
CS_DATABASE__USERNAME
CS_DATABASE__PASSWORD
CS_AUTH__WORKSPACE_ID
CS_AUTH__CLIENT_ACCESS_KEY
CS_ENCRYPT__DATASET_ID
CS_ENCRYPT__CLIENT_ID
CS_ENCRYPT__CLIENT_KEY
```

Read the full list of environment variables and what they do in the [reference documentation](#proxy-config-options).

#### Configuring Proxy with a TOML file

If you are configuring Proxy with a `cipherstash-proxy.toml` file, these are the minimum values required to run Proxy:

```toml
[database]
name = "cipherstash"
username = "cipherstash"
password = "password"

[auth]
workspace_id = "cipherstash-workspace-id"
client_access_key = "cipherstash-client-access-key"

[encrypt]
dataset_id = "cipherstash-dataset-id"
client_id = "cipherstash-client-id"
client_key = "cipherstash-client-key"
```

Read the full list of configuration options and what they do in the [reference documentation](#proxy-config-options).

### Running Proxy locally

TODO: Add instructions for running Proxy locally

### Setting up the database schema

TODO: Add instructions for setting up the database schema

#### Creating columns with the right types

TODO: Add instructions for creating columns with the right types

## Reference

This section contains reference documentation for configuring CipherStash Proxy and its features.

### Proxy config options

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
# Sets how long to hold an open connection
# Optional
# Default: `300000` (5 minutes)
# Env: CS_DATABASE__SHUTDOWN_TIMEOUT
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
# Env: CS_TLS__CERTIFICATE
certificate = "./server.cert"

# Private Key path
# Env: CS_TLS__PRIVATE_KEY
private_key = "./server.key"


[auth]
# CipherStash Workspace ID
# Env: CS_AUTH__WORKSPACE_ID
workspace_id = "cipherstash-workspace-id"

# CipherStash Client Access Key
# Env: CS_AUTH__CLIENT_ACCESS_KEY
client_access_key = "cipherstash-client-access-key"

[encrypt]
# CipherStash Dataset ID
# Env: CS_ENCRYPT__DATASET_ID
dataset_id = "cipherstash-dataset-id"

# CipherStash Client ID
# Env: CS_ENCRYPT__CLIENT_ID
client_id = "cipherstash-client-id"

# CipherStash Client Key
# Env: CS_ENCRYPT__CLIENT_KEY
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
# Default: `pretty`
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
# Default: `true`
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


### Prometheus metrics

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


#### Available metrics

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


## More info

### Developing for Proxy

Check out the [Proxy development guide](./DEVELOPMENT.md).
