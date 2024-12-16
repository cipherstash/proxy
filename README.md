# CipherStash Proxy

CipherStash Proxy keeps your sensitive data in PostgreSQL encrypted and searchable, without changing your SQL queries.

Behind the scenes, it uses the [Encrypt Query Language](https://github.com/cipherstash/encrypt-query-language/) to index and search encrypted data.

## Developing

> [!IMPORTANT]
> **Before you start:** you need to have this software installed:
>  - [Rust](https://www.rust-lang.org/)
>  - [mise](https://mise.jdx.dev/)
>  - [Docker](https://www.docker.com/)

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

### Building

> [!IMPORTANT]
> **Before you start:** ensure you have an SSH authentication key [added to your GitHub account](https://github.com/settings/keys).

To build a binary for Proxy, run:

```bash
cargo build
```

### Dependencies

Configure `Auth` and `Encrypt`

Using environment variables:
Copy `mise.local.example.toml` to `mise.local.toml` and edit

Using toml:
Copy `cipherstash-proxy-example.toml` to `cipherstash-proxy.toml` and edit.


```shell
# install development and testing dependencies (including cargo-binstall)
mise install

# start all postgres instances
mise run up

# install latest eql into database
mise run setup

# build and run the proxy
mise run proxy
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

All the data directories for the Docker container live in `tests/pg/data-*`.

They are ephemeral, and ignored in `.gitignore`.

Sometimes the PostgreSQL instances get into an inconsistent state, and need to be reset.
To wipe all PostgreSQL data directories:

```
mise run destroy_data
```

## Prerequisites

- [mise](https://mise.jdx.dev/)
- [Docker](https://www.docker.com/)
- [Rust](https://www.rust-lang.org/)
- [PostgreSQL](https://www.postgresql.org/)

PostgreSQL database configuration is defined in `tests/docker-compose.yml'
See `Docker Compose` below for details.

- [Bininstall](https://github.com/cargo-bins/cargo-binstall)
- [Mise](https://github.com/jdxcode/mise)
- [Nextest](https://nexte.st/)
- [Docker](https://www.docker.com/)
- [Docker Compose](https://docs.docker.com/compose/)



### Tests

> [!IMPORTANT]
> **Before you start:** ensure you have [Nextest](https://nexte.st/) installed:
> ```bash
> cargo binstall cargo-nextest --secure
> ```

To set up your local development environment, run:

```
mise run setup
```

Assumes running docker postgres service with default credentials

To run all tests:

```bash
mise run test
```

To run a single test:

```bash
mise run test {TEST_NAME}
mise run test load_schema
```

> [!TIP]
> Mise provides abbreviations for most of the commands above.
> For example, `mise run setup` can be abbreviated to `mise r s`.
> Check out `mise.toml` for all the task shortcuts we have defined.

### Docker Compose

Conventions for running multiple postgres versions

The goal is to have as little to configure in local dev as possible.

To run all services:
```bash
mise run up
```

To run a specific service
```bash
mise run up postgres
```

### common configuration

All containers use the same credentials and database, defined in `pg/common.env`

```
POSTGRES_DB="cipherstash"
POSTGRES_USER="cipherstash"
PGUSER="cipherstash"
POSTGRES_PASSWORD="password"
```

### ports


Vanilla connection ports start with `55` followed by the `version` number
TLS connection ports start with `56` followed by the `version` number

Postgres latest always runs on `5532`

```
    55{version}

    # v17
    5517

    # v17 with TLS
    5617
```


### container_name
```
    pg-{version}
    pg-{version}-tls

    pg-17
    pg-17-tls
```


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
