# CipherStash Proxy

CipherStash Proxy keeps your sensitive data in PostgreSQL encrypted and searchable, without changing your SQL queries.

Behind the scenes, it uses the [Encrypt Query Language](https://github.com/cipherstash/encrypt-query-language/) to index and search encrypted data.

## Developing

> [!IMPORTANT]
> **Before you quickstart** you need to have this software installed:
>  - [Rust](https://www.rust-lang.org/)
>  - [mise](https://mise.jdx.dev/) - see the [installing mise](#installing-mise) instructions
>  - [Docker](https://www.docker.com/)

Local development quickstart:

```shell
# Clone the repo
git clone https://github.com/cipherstash/proxy
cd proxy

# Install dependencies
mise trust --yes
mise install

# Start all postgres instances
mise run postgres:up --extra-args "--detach --wait"

# Install latest eql into database
mise run setup

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

# Run tests
mise run test
```

### How this project is organised

Development is managed through [mise](https://mise.jdx.dev/), both locally and [in CI](https://github.com/cipherstash/proxy/actions).

mise has tasks for:

- Starting and stopping PostgreSQL containers (`postgres:up`, `postgres:down`)
- Running hygiene tests (`test:check`, `test:clippy`, `test:format`)
- Running unit tests (`test:unit`)
- Running integration tests (`test:integration`, `test:integration:*`)
- Running tests in CI (`test:ci`)
- Building binaries (`build:binary`) and Docker images (`build:docker`)
- Publishing release artifacts (`release`)

These are the important files in the repo:

```
.
├── mise.toml              <-- the main config file for mise
├── mise.local.toml        <-- optional overrides for local customisation of mise
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
echo 'eval "$(~/.local/bin/mise activate bash)"' >> ~/.bashrc

# If you're running Zsh
echo 'eval "$(~/.local/bin/mise activate zsh)"' >> ~/.zshrc
```

We use [`cargo-binstall`](https://github.com/cargo-bins/cargo-binstall) for faster installation of tools installed via `mise` and Cargo.
We install `cargo-binstall` via `mise` when installing development and testing dependencies.

> [!TIP]
> We provide abbreviations for most of the commands that follow.
> For example, `mise run setup` can be abbreviated to `mise r s`.
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
mise run up
```

Then set up the schema and functions:

```shell
# install latest eql into database
mise run setup
```

You can start PostgreSQL containers in a couple of different ways:

```
# Start all postgres instance in the foreground
mise run up
# exit by hitting ctrl + c

# Start postgres instances individually in the foreground
mise run up postgres
mise run up postgres-17-tls

# Start a postgres instance in the background
mise run up postgres --extra-args "--detach --wait"

# Stop and remove all containers, networks, and postgres data
mise run down
```

Configuration for starting PostgreSQL instances is in `tests/docker-compose.yml`

All the data directories for the Docker container live in `tests/pg/data-*`.
They are ephemeral, and ignored in `.gitignore`.

Sometimes the PostgreSQL instances get into an inconsistent state, and need to be reset.
To wipe all PostgreSQL data directories:

```
mise run destroy_data
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

### Building and running

Build and run Proxy locally:

```shell
mise run proxy
```

Build a standalone releasable binary for Proxy:

```bash
mise run build:binary
```

### Tests

FIXME: document docker + integration tests
Assumes running docker postgres service with default credentials

To run all tests:

```bash
mise run test
```

To run a single unit test:

```bash
mise run test:unit <TEST_NAME>

# For example:
mise run test:unit test_database_as_url
```

There are individual hygiene tests you can run:

```shell
# check everything compiles
mise run test:check

# run clippy lints
mise run test:clippy

# check rust is formatted correctly
mise run test:format
```

To run integration tests:

```
mise run test:integration
```

The integration tests have several runtime dependencies:

- Running PostgreSQL instances
- Credentials for CipherStash ZeroKMS

### common configuration

All containers use the same credentials and database, defined in `pg/common.env`

```
POSTGRES_DB="cipherstash"
POSTGRES_USER="cipherstash"
PGUSER="cipherstash"
POSTGRES_PASSWORD="password"
```

### Conventions

#### PostgreSQL Ports

PostgreSQL port numbers are 4 digits:

- The first two digits denote non-TLS (`55`) or non-TLS (`56`)
- The last two digits denote the version of PostgreSQL

PostgreSQL latest always runs on `5532`.

These are the Postgres instances and ports currently provided:

| Port   | Description                    |
|--------|--------------------------------|
| `5617` | TLS, PostgreSQL version 17     |
| `5532` | non-TLS, Postgres latest       |


### PostgreSQL container names

Container names are in the format:

> `postgres-<version>[-tls]`

These are the Postgres instances and names currently provided:

| Name              | Description                    |
|-------------------|--------------------------------|
| `postgres-17-tls` | TLS, PostgreSQL version 17     |
| `postgres`        | non-TLS, Postgres latest       |


### config files
```
    ./pg/postgresql-tls.conf
    ./pg/pg_hba-tls.conf
```

Configuration is quite consistent between versions and we shouldn't need many version-specific configurations.


### data directory

Mount the data directory to access logs.

```
    .pg/data-{version}
```


## TLS

### Configuration

Turns ssl on and enforces `md5` password hashing.
- pg/postgresql-tls.conf
- pg/pg_hba-tls.conf


Uses certs generated by mkcert
- tls/localhost-key.pem
- tls/localhost-key.pem


## Commands

Connect to pg 17 over TLS
```
docker compose up --build
```




## Integration Tests

Integration tests require a running proxy, which requires a running PostgreSQL database.

Integration tests should be named in the form `integration_{test_name}` because I could not work out another way of filtering.

### psql connection tests

Connecting to the proxy via psql is handled via mise environments and files tasks.

Running mise from the test directory will prioritise config in the test directory, and mise will load a specified environment using the `--env` arg.

```shell
cd tests/

# run the proxy in background
mise --env tcp run proxy &

# run psql tcp connection tests
mise --env tcp r test:psql-tcp
```

Task files:

 - `tests/tasks/test/psql-tcp.sh`
 - `tests/tasks/test/psql-tls.sh`

Environment files:
 - `test/mise.toml`
 - `test/mise.tcp.toml`
 - `test/mise.tls.toml`


### `test/mise.toml`
Database credentials, assumes the default credentials used by the `docker-compose` services.

Note: does not include a database port. The port is defined in the named environment files.


### `test/mise.tcp.toml`

Points to the latest PostgreSQL service at `5532`.


### `test/mise.tls.toml`

Points to the PostgreSQL 17 with TLS Service at `5617`.

Configures the TLS certificate, private key and ensures the server requires tls.
