# CipherStash Proxy

CipherStash Proxy provides a transparent proxy to your existing postgres database, handling the complexity of encrypting and decrypting your data.
CipherStash Proxy keeps your sensitive data in PostgreSQL encrypted and searchable, without changing your SQL queries.

Behind the scenes, it uses the [Encrypt Query Language](https://github.com/cipherstash/encrypt-query-language/) to index and search encrypted data.


## Getting Started


## Configuration

To run, CipherStash Proxy needs to know:

- What port to run on
- How to connect to the target PostgreSQL database
- Secrets to authenticate to CipherStash

CipherStash Proxy can source configuration from a config file, and environment variables:

- If `cipherstash-proxy.toml` is present in the current working directory, Proxy will read its config from that file
- If `cipherstash-proxy.toml` is not present, Proxy will look up environment variables to configure itself
- If both `cipherstash-proxy.toml` and environment variables are present, Proxy will use `cipherstash-proxy.toml` as the base configuration, and override it with any environment variables that are set

Example configuration files are in [`cipherstash-proxy-example.toml`](./cipherstash-proxy-example.toml) and [`./packages/cipherstash-proxy/tests/config/`](./packages/cipherstash-proxy/tests/config/).

If you are configuring Proxy with a `cipherstash-proxy.toml` file, these are the minimum values required to run Proxy:

```toml
[database]
name = "cipherstash"
username = "cipherstash"
password = "password"

[auth]
workspace_id = "cipherstash-workspace-id"
client_access_key = "cipherstash-clien-access-key"

[encrypt]
dataset_id = "cipherstash-dataset-id"
client_id = "cipherstash-client-id"
client_key = "cipherstash-client-key"
```

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

### Configuration Options

These are all the configuration options available for Proxy:

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
require_tls = "false",

# Shutdown timeout in ms
# Sets how long to wait for connections to drain on shutdown
# Optional
# Default: `2000`
# Env: CS_SERVER__SHUTDOWN_TIMEOUT
shutdown_timeout = "2000"


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
name = "5432"

# Database username
# Env: CS_DATABASE__USERNAME
username = "username"

# Database username
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
# Cipherstash Workspace Id
# Env: CS_AUTH__WORKSPACE_ID
workspace_id = "cipherstash-workspace-id"

# Cipherstash Client Access Key
# Env: CS_AUTH__CLIENT_ACCESS_KEY
client_access_key = "cipherstash-client-access-key"

[encrypt]
# Cipherstash Dataset Id
# Env: CS_AUTH__DATASET_ID
dataset_id = "cipherstash-dataset-id"

# Cipherstash Client Id
# Env: CS__AUTH__cipherstash__CLIENT__ID
client_id = "cipherstash-client-id"

# Cipherstash Client Key
# Env: CS_AUTH__CLIENT_KEY
client_key = "cipherstash-client-key"


[log]
# Log level
# Optional
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

```





## Developing

> [!IMPORTANT]
> **Before you quickstart** you need to have this software installed:
>  - [mise](https://mise.jdx.dev/) — see the [installing mise](#installing-mise) instructions
>  - [Docker](https://www.docker.com/) — see Docker's [documentation for installing](https://docs.docker.com/get-started/get-docker/)

Local development quickstart:

```shell
# Clone the repo
git clone https://github.com/cipherstash/proxy
cd proxy

# Double check you have all the development dependencies
sh preflight.sh

# Install dependencies
mise trust --yes
mise trust --yes tests
mise install

# Start all postgres instances
mise run postgres:up --extra-args "--detach --wait"

# Install latest eql into database
mise run postgres:setup

# If this is your first time using CipherStash:
#  - install stash CLI
#  - `stash signup`

# If you have used CipherStash before:
#  - `stash login`

# Create minimal mise.local.toml
# CS_AUTH__WORKSPACE_ID
# CS_AUTH__CLIENT_ACCESS_KEY
# CS_ENCRYPT__DATASET_ID
# CS_ENCRYPT__CLIENT_KEY
# CS_ENCRYPT__CLIENT_ID

# Get the workspace ID
stash workspaces
# add to CS_AUTH__WORKSPACE_ID

# Create an access key
stash access-keys create proxy
# add to CS_AUTH__CLIENT_ACCESS_KEY

# Create a dataset
stash datasets create proxy
# add to CS_ENCRYPT__DATASET_ID

# Create a client
stash clients create --dataset-id $DATASET_ID proxy
# add to CS_ENCRYPT__CLIENT_ID
# add to CS_ENCRYPT__CLIENT_KEY

# Build and run Proxy
mise run proxy
# exit by hitting ctrl + c

# Run tests
mise run test
```

### How this project is organised

Development is managed through [mise](https://mise.jdx.dev/), both locally and [in CI](https://github.com/cipherstash/proxy/actions).

mise has tasks for:

- Starting and stopping PostgreSQL containers (`postgres:up`, `postgres:down`)
- Starting and stopping Proxy as a process or container (`proxy`, `proxy:up`, `proxy:down`)
- Running hygiene tests (`test:check`, `test:clippy`, `test:format`)
- Running unit tests (`test:unit`)
- Running integration tests (`test:integration`, `test:integration:*`)
- Building binaries (`build:binary`) and Docker images (`build:docker`)
- Publishing release artifacts (`release`)

These are the important files in the repo:

```
.
├── mise.toml              <-- the main config file for mise
├── mise.local.toml        <-- optional overrides for local customisation of mise
├── proxy.Dockerfile       <-- Dockerfile for building CipherStash Proxy image
├── packages/              <-- Rust packages used to make CipherStash Proxy
├── target/                <-- Rust build artifacts
└── tests/                 <-- integration tests
    ├── docker-compose.yml <-- Docker configuration for running PostgreSQL instances
    ├── mise*.toml         <-- environment variables used by integration tests
    ├── pg/                <-- data and configuration for PostgreSQL instances
    ├── sql/               <-- SQL used to initialise PostgreSQL instances
    ├── tasks/             <-- mise file tasks, used for integration tests
    └── tls/               <-- key material for testing TLS with PostgreSQL
```

> [!IMPORTANT]
> **Before you start developing:** you need to have this software installed:
>  - [Rust](https://www.rust-lang.org/)
>  - [mise](https://mise.jdx.dev/)
>  - [Docker](https://www.docker.com/)

### Installing mise

> [!IMPORTANT]
> You must complete this step to set up a local development environment.

Local development is managed through [mise](https://mise.jdx.dev/).

To install mise:

- If you're on macOS, run `brew install mise`
- If you're on another platform, check out the mise [installation methods documentation](https://mise.jdx.dev/installing-mise.html#installation-methods)

Then add mise to your shell:

```
# If you're running Bash
echo 'eval "$(mise activate bash)"' >> ~/.bashrc

# If you're running Zsh
echo 'eval "$(mise activate zsh)"' >> ~/.zshrc
```

We use [`cargo-binstall`](https://github.com/cargo-bins/cargo-binstall) for faster installation of tools installed via `mise` and Cargo.
We install `cargo-binstall` via `mise` when installing development and testing dependencies.

> [!TIP]
> We provide abbreviations for most of the commands that follow.
> For example, `mise run postgres:setup` can be abbreviated to `mise r s`.
> Run `mise tasks --extended` to see the task shortcuts.

### Rust dependencies

> [!IMPORTANT]
> You must complete this step to set up a local development environment.

Install development and testing dependencies (including `cargo-binstall`):

```shell
mise install
```

### PostgreSQL

> [!IMPORTANT]
> You must complete this step to set up a local development environment.

We ship containers for running PostgreSQL in different configurations.
The goal is to minimise the amount of configuration you have to do in local dev.

To start all PostgreSQL instances:

```shell
mise run postgres:up
```

Then set up the schema and functions:

```shell
# install latest eql into database
mise run postgres:setup
```

You can start PostgreSQL containers in a couple of different ways:

```
# Start all postgres instance in the foreground
mise run postgres:up
# exit by hitting ctrl + c

# Start postgres instances individually in the foreground
mise run postgres:up postgres
mise run postgres:up postgres-17-tls

# Start a postgres instance in the background
mise run postgres:up postgres --extra-args "--detach --wait"

# Stop and remove all containers, networks, and postgres data
mise run postgres:down
```

Configuration for starting PostgreSQL instances is in `tests/docker-compose.yml`

All the data directories for the Docker container live in `tests/pg/data-*`.
They are ephemeral, and ignored in `.gitignore`.

Sometimes the PostgreSQL instances get into an inconsistent state, and need to be reset.
To wipe all PostgreSQL data directories:

```
mise run postgres:destroy_data
```

### Proxy configuration

> [!IMPORTANT]
> You must complete this step to set up a local development environment.

There are two ways to configure Proxy:

- Environment variables that Proxy looks up on startup
- TOML file that Proxy reads on startup

To configure Proxy with environment variables:

```
cp mise.local.example.toml mise.local.toml
$EDITOR mise.local.toml
```

Configure `Auth` and `Encrypt`

To configure Proxy with a TOML file:

```
cp cipherstash-proxy-example.toml cipherstash-proxy.toml
$EDITOR cipherstash-proxy.toml
```

Configure `Auth` and `Encrypt`

#### Logging configuration

Logging can be configured by setting appropriate environment variables.

There are "levels" and "targets" in Proxy logging configuration.
The levels set the verbosity. The possible values are:

- trace
- debug
- info
- warn
- error

A Proxy-wide default level is configured by setting the environment variable `RUST_LOG`.
If this variable is not set, the default value set in the Proxy code will be used.

There are different "log targets" in Proxy.
They correspond to modules or functionalities.
Set log levels for a specific log target to turn on or turn of more verbose logging.

> [!IMPORTANT]
> The application code must use the 'target' parameter for the per-target log level to work.
> An example: `debug!(target: AUTHENTICATION, "SASL authentication successful");`

### Running Proxy locally

There are two ways to run Proxy locally:

1. **As a process**, most useful for local development
1. **In a container**, used for integration tests

#### Running Proxy locally as a process

Build and run Proxy as a process on your local machine:

```shell
mise run proxy
# exit by hitting ctrl + c
```

Kill any Proxy processes running in the background:

```shell
mise run proxy:kill
```

#### Running Proxy locally in a container

To run Proxy in a container on your local machine:

```shell
mise run proxy:up
# exit by hitting ctrl + c
```

There are two different Proxy containers you can run:

1. One with TLS (`proxy-tls`)
1. One without TLS (`proxy`)

```shell
# Default `proxy:up` behaviour starts the TLS-less Proxy container
mise run proxy:up

# Explicitly starting the TLS-less Proxy container
mise run proxy:up proxy

# Start the Proxy container with TLS
mise run proxy:up proxy-tls
```

You can pass extra arguments to `proxy:up`, to run the Proxy container in the background:

```shell
mise run proxy:up --extra-args "--detach --wait"
```

Any options you pass via `--extra-args` will be passed to `docker compose up` behind the scenes.

When you have Proxy containers running in the background, you can stop them with this:

```shell
mise run proxy:down
```

Running Proxy in a container cross-compiles a binary for Linux and the current architecture (`amd64`, `arm64`), then copies the binary into the container.
We cross-compile binary outside the container because it's generally faster, due to packages already being cached, and slower network and disk IO in Docker.

### Building

Build a binary and Docker image:

```shell
mise run build
```

You can also do those two steps individually.

Build a standalone releasable binary for Proxy, for the current architecture and operating system:

```shell
mise run build:binary
```

Build a Docker image using the binary produced from running `mise run build:binary`:

```shell
# build an image for the current architecture and operating system
mise run build:docker

# build an image for a specific architecure and operating system
mise run build:docker --platform linux/arm64
```

### Tests

There is a wide range of tests for Proxy:

- Hygiene tests (`test:check`, `test:clippy`, `test:format`)
- Unit tests (`test:unit`)
- Integration tests (`test:integration`, `test:integration:*`)

To run all tests:

```shell
# run the full test suite
mise run test
```

To run hygiene tests:

```shell
# check everything compiles
mise run test:check

# run clippy lints
mise run test:clippy

# check rust is formatted correctly
mise run test:format
```

To run unit tests:

```shell
# run all unit tests
mise run test:unit

# run a single unit test
mise run test:unit test_database_as_url
```

To run integration tests:

```
mise run test:integration
```

#### Integration tests

Integration tests are defined in `tests/tasks/test/`.

Integration tests verify behaviour from a PostgreSQL client (`psql`), to Proxy (running in a container), to a PostgreSQL instance (running in a container).

The integration tests have several runtime dependencies:

- Running PostgreSQL instances (that can be started with `mise run postgres:up`)
- Credentials for CipherStash ZeroKMS (which can be found in the [quickstart](#developing) section)


### Working with Encrypt Query Language (EQL)

The [Encrypt Query Language (EQL)](https://github.com/cipherstash/encrypt-query-language/) is a set of abstractions for transmitting, storing, and interacting with encrypted data and indexes in PostgreSQL.

EQL is a required dependency and the database setup uses the latest release.

To use a different version of EQL, set the path to the desired EQL release file in the `CS_EQL_PATH` environment variable.



#### Convention: PostgreSQL ports

PostgreSQL port numbers are 4 digits:

- The first two digits denote non-TLS (`55`) or non-TLS (`56`)
- The last two digits denote the version of PostgreSQL

PostgreSQL latest always runs on `5532`.

These are the Postgres instances and ports currently provided:

| Port   | Description                    |
|--------|--------------------------------|
| `5617` | TLS, PostgreSQL version 17     |
| `5532` | non-TLS, Postgres latest       |


#### Convention: PostgreSQL container names

Container names are in the format:

> `postgres-<version>[-tls]`

These are the Postgres instances and names currently provided:

| Name              | Description                    |
|-------------------|--------------------------------|
| `postgres-17-tls` | TLS, PostgreSQL version 17     |
| `postgres`        | non-TLS, Postgres latest       |


#### Configuration: integration test PostgreSQL and Proxy containers

This project uses `docker compose` to manage containers and networking.

The configuration for those containers is in `tests/docker-compose.yml`.

The integration tests use the `proxy:up` and `proxy:down` commands documented above to run containers in different configurations.

#### Configuration: configuring PostgreSQL containers in integration tests

All containers use the same credentials and database, defined in `tests/pg/common.env`

```
POSTGRES_DB="cipherstash"
POSTGRES_USER="cipherstash"
PGUSER="cipherstash"
POSTGRES_PASSWORD="password"
```

PostgreSQL configuration files live at:

- PostgreSQL with TLS: `tests/pg/postgresql-tls.conf`, `tests/pg/pg_hba-tls.conf`
- PostgreSQL without TLS: default configuration shipped with [`postgres` containers](https://hub.docker.com/_/postgres)

Configuration is quite consistent between versions and we shouldn't need many version-specific configurations.

#### Convention: integration test PostgreSQL containers data directories

The data directories used by the PostgreSQL containers can be found at `tests/pg/data-*`.

These directories contain logs and other useful debugging information.

#### Configuration: integration test environment files

Environment files:

- `tests/mise.toml` - default database credentials used by the `docker-compose` services. Does not include a database port (`CS_DATABASE__PORT`). The port is defined in the named environment files.
- `tests/mise.tcp.toml` - credentials used for non-TLS integration tests.
- `tests/mise.tls.toml` -  credentials used for TLS integration tests. Configures the TLS certificate and private key, and ensures the server requires TLS.

If you ever get confused about where your configuration is coming from, run `mise cfg` to get a list of config files in use.

Certificates are generated by `mkcert`, and live in `tests/tls/`.
