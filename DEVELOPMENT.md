# Developing CipherStash Proxy

> [!IMPORTANT]
> **Before you quickstart** you need to have this software installed:
>  - [mise](https://mise.jdx.dev/) — see the [installing mise](#installing-mise) instructions
>  - [Docker](https://www.docker.com/) — see Docker's [documentation for installing](https://docs.docker.com/get-started/get-docker/) and ensure that your Docker daemon is running. On *macOS* and *Linux* you can do this by running Docker Desktop.  See: [Docker Desktop docs](https://docs.docker.com/get-started/introduction/get-docker-desktop/). If you have installed Docker without Docker Desktop then you will need to launch `dockerd` manually.

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

# Create a stub mise.local.toml
cat > mise.local.toml << 'EOF'
[env]
CS_WORKSPACE_CRN = ""
CS_CLIENT_KEY = ""
CS_CLIENT_ID = ""
CS_CLIENT_ACCESS_KEY = ""
CS_DEFAULT_KEYSET_ID = ""
EOF

# In your browser:
#  - Sign in to https://dashboard.cipherstash.com
#  - Create or select a workspace
#  - Generate and copy the credentials to your clipboard

# In your terminal:
#  - Paste the credentials into mise.local.toml using your preferred text editor
nano mise.local.toml

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
mise run postgres:up postgres-tls

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

Logging can be configured by setting environment variables.

There are "levels" and "targets" in Proxy logging configuration.
The levels set the verbosity.

The possible values are:

- `trace`
- `debug`
- `info`
- `warn`
- `error`

The Proxy-wide default level can be configured by `CS_LOG__LEVEL`.
Default level is `Info`.

Proxy has multiple "log targets" corresponding to the internal domains.

Set log levels for a specific log target to turn on or turn off more verbose logging:

```
Target          | ENV
--------------- | -------------------------------------
DEVELOPMENT     | CS_LOG__DEVELOPMENT_LEVEL
AUTHENTICATION  | CS_LOG__AUTHENTICATION_LEVEL
CONFIG          | CS_LOG__CONFIG_LEVEL
CONTEXT         | CS_LOG__CONTEXT_LEVEL
ENCODING        | CS_LOG__ENCODING_LEVEL
ENCRYPT         | CS_LOG__ENCRYPT_LEVEL
DECRYPT         | CS_LOG__DECRYPT_LEVEL
ENCRYPT_CONFIG  | CS_LOG__ENCRYPT_CONFIG_LEVEL
KEYSET          | CS_LOG__KEYSET_LEVEL
MIGRATE         | CS_LOG__MIGRATE_LEVEL
PROTOCOL        | CS_LOG__PROTOCOL_LEVEL
PROXY           | CS_LOG__PROXY_LEVEL
MAPPER          | CS_LOG__MAPPER_LEVEL
SCHEMA          | CS_LOG__SCHEMA_LEVEL
```


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

The `Multitenant` Integration tests require different configuration from the baseline.
The `CS_DEFAULT_KEYSET_ID` value must not be set for the multitenant `SET KEYSET_*` commands to work.

##### Language-specific integration tests

To run language-specific integration tests, call:

- `mise run test:integration:lang:golang`
- `mise run test:integration:lang:python`


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
| `postgres-tls`    | TLS, (PostgreSQL version 17)   |
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


#### Configuration: development endpoints


ZeroKMS and CTS host endpoints can be configured for local development.

Env variables are `CS_DEVELOPMENT__ZEROKMS_HOST` and `CS_DEVELOPMENT__CTS_HOST`.


```toml

[development]
# ZeroKMS host
# Optional
# Defaults to CipherStash Production ZeroKMS host
# Env: CS_DEVELOPMENT__ZEROKMS_HOST
zerokms_host = "1.1.1.1"


# CTS host
# Optional
# Defaults to CipherStash Production CTS host
# Env: CS_DEVELOPMENT__CTS_HOST
cts_host = "1.1.1.1"

```





## Logging

- Use structured logging
- Use the appropriate targets
- Include the `client_id` where appropriate

Debug logging is very verbose, and targets allow configuration of granular log levels.

A `target` is a string value that is added to the standard tracing macro calls (`debug!, error!, etc`).
Log levels can be configured for each `target` individually.

### Logging Architecture

The logging system uses a declarative macro approach for managing log targets:

- **Single source of truth**: All log targets are defined in `packages/cipherstash-proxy/src/log/targets.rs` using the `define_log_targets!` macro
- **Automatic generation**: The macro generates target constants, configuration struct fields, and accessor functions
- **Type safety**: Uses a generated `LogTargetLevels` struct with serde flattening for config integration
- **Self-documenting**: Clear separation between core config and target-specific levels

The targets are aligned with the different components and contexts (`PROTOCOL`, `AUTHENTICATION`, `MAPPER`, etc.).

There is a general `DEVELOPMENT` target for logs that don't quite fit into a specific category.

### Adding a New Log Target

To add a new log target, you only need to add **one line** to the macro in `packages/cipherstash-proxy/src/log/targets.rs`:

```rust
define_log_targets!(
    (DEVELOPMENT, development_level),
    (AUTHENTICATION, authentication_level),
    // ... existing targets ...
    (NEW_TARGET, new_target_level), // <- Add this line
);
```

The constant name (e.g., `NEW_TARGET`) is automatically converted to a string value using `stringify!()`, so `NEW_TARGET` becomes `"NEW_TARGET"` for use in logging calls.

This automatically:
- Creates the `NEW_TARGET` constant for use in logging calls
- Generates the `new_target_level` field in `LogTargetLevels` struct
- Creates the environment variable `CS_LOG__NEW_TARGET_LEVEL`
- Adds the target to all logging functions and validation

### Available targets

```
Target          | ENV
--------------- | -------------------------------------
DEVELOPMENT     | CS_LOG__DEVELOPMENT_LEVEL
AUTHENTICATION  | CS_LOG__AUTHENTICATION_LEVEL
CONFIG          | CS_LOG__CONFIG_LEVEL
CONTEXT         | CS_LOG__CONTEXT_LEVEL
ENCODING        | CS_LOG__ENCODING_LEVEL
ENCRYPT         | CS_LOG__ENCRYPT_LEVEL
DECRYPT         | CS_LOG__DECRYPT_LEVEL
ENCRYPT_CONFIG  | CS_LOG__ENCRYPT_CONFIG_LEVEL
KEYSET          | CS_LOG__KEYSET_LEVEL
MIGRATE         | CS_LOG__MIGRATE_LEVEL
PROTOCOL        | CS_LOG__PROTOCOL_LEVEL
PROXY           | CS_LOG__PROXY_LEVEL
MAPPER          | CS_LOG__MAPPER_LEVEL
SCHEMA          | CS_LOG__SCHEMA_LEVEL
```

### Example

The default log level for the proxy is `info`.

An `env` variable can be used to configure the logging level.

Configure `debug` for the `MAPPER` target:

```shell
CS_LOG__MAPPER_LEVEL = "debug"
```

Log `debug` output for the `MAPPER` target:

```rust
debug!(
    target: MAPPER,
    client_id = self.context.client_id,
    identifier = ?identifier
);
```

### Implementation Details

The logging system uses these key components:

- **`define_log_targets!` macro** in `log/targets.rs`: Generates all logging infrastructure
- **`LogTargetLevels` struct**: Auto-generated struct containing all target level fields
- **`LogConfig` struct**: Main configuration with `#[serde(flatten)]` integration
- **Target constants**: Exported for use in logging calls (e.g., `MAPPER`, `PROTOCOL`)
- **Environment variables**: Auto-generated from target names (e.g., `CS_LOG__MAPPER_LEVEL`)

## Benchmarks

Benchmarking is integrated into CI, and can be run locally.

The benchmark uses `pgbench` to compare direct access to PostgreSQL against `pgbouncer` and Proxy.
Benchmarks are executed in Docker containers, as the focus is on providing a repeatable baseline for performance.
Your mileage may vary when comparing against a "real-world" production configuration.

The benchmarks use the extended protocol option and cover:
- the default `pgbench` transaction
- insert, update & select of plaintext data
- insert, update & select of encrypted data (CipherStash Proxy only)

The benchmark setup includes the database configuration, but does requires access to a CipherStash account environment.

### Required environment variables
```
CS_WORKSPACE_CRN
CS_CLIENT_ACCESS_KEY
CS_DEFAULT_KEYSET_ID
CS_CLIENT_ID
CS_CLIENT_KEY
```


### Connecting to another environment

Proxy connects to the CipherStash production environment by default.

To connect to an alternative environment, provide the CTS and ZeroKMS host endpoints:

```
CS_CTS_HOST
CS_ZEROKMS_HOST
```


### Running the benchmark

```bash
cd tests/benchmark
mise run benchmark
```

Results are graphed in a file called `benchmark-{YmdHM}.png` where `YmdHM` is a generated timestamp.
Detailed results are generated in `csv` format and in the `results` directory.

## Style Guide

### Testing

#### Use `unwrap()` instead of `expect()` unless providing context
When working with `Result` and `Option` in Rust tests, prefer `unwrap()` over `expect()` unless the error message provides meaningful context.
While both are functionally equivalent, `expect()` can introduce unnecessary noise if its message is generic.
If additional context is necessary, use `expect()` with a clear explanation of why the value should be `Ok` or `Some`.

Reference: [Rust documentation on `expect`](https://doc.rust-lang.org/std/result/enum.Result.html#method.expect)

#### Prefer `assert_eq!` over `assert!` for equality checks
Use `assert_eq!` instead of `assert!` when testing equality in Rust.
While both achieve the same result, `assert_eq!` provides clearer failure messages by displaying the expected and actual values, making debugging easier.



### Errors

- errors are defined in `cipherstash-proxy/src/error.rs`
- not all errors are customer-facing
- for all customer-facing errors ensure there is:
 - a friendly error message
 - an entry in `docs/errors.md`
 - a link in the error message to the appropraite anchor in `docs/errors.md`

#### Keep the errors in one place

Keeping the errors in one place mean that the error structure is
- easier to visualise and understand
- simpler to deduplicate for consistent messaging
- easier to edit messages for consistent tone


#### Align errors to the domain, not the module structure

The error structure attempts to group errors into related problem domains.
eg, the `Protocol` error groups all of the errors originating from interactions with the Postgresql protocol and is used by any module that interacts with the Protocol.
If each module defines errors, it becomes harder to have consistent messaging and more difficult to understand the error flow.


#### Prefer descriptive names and don't use Error

Be kind to your future self and make the error as descriptive as possible

eg `ColumnCouldNotBeEncrypted` over `ColumnError`

Error enum names should contain `Error` but the variants should not.

The enum defines the `Error` for the domain, and the variant describes the error.

eg `ColumnCouldNotBeEncrypted` is a variant of an `EncryptError`.


#### Make errors as friendly as possible, include details and keep a consistent tone

Friendly can be hard when talking about errors, but do your best.


#### Prefer struct definitions

Struct definitions make error message strings slightly clearer.

`UnsupportedParameterType { name: String, oid: u32 }` over `UnsupportedParameterType(String, u32)`

Note: not all errors do this at the moment, and we will change over time.

