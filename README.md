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
# install nextest and other rust dependencies
mise run install

# run postgres on default port
mise run up postgres

# install latest eql into database
mise run setup

mise run proxy
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

```
psql postgresql://cipherstash:password@localhost:5517/cipherstash
```
