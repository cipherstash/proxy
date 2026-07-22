# CipherStash Proxy how-to guide

This page contains how-to documentation for installing, configuring, and running CipherStash Proxy.

## Table of contents

- [Installing Proxy](#installing-proxy)
- [Configuring Proxy](#configuring-proxy)
  - [Configuring Proxy with environment variables](#configuring-proxy-with-environment-variables)
  - [Configuring Proxy with a TOML file](#configuring-proxy-with-a-toml-file)
- [Running Proxy locally](#running-proxy-locally)
- [Setting up the database schema](#setting-up-the-database-schema)
  - [Creating columns with the right types](#creating-columns-with-the-right-types)
- [Encrypting data in an existing database](#encrypting-data-in-an-existing-database)

## Installing Proxy

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
      # The CipherStash workspace CRN for making requests for encryption keys
      - CS_WORKSPACE_CRN=${CS_WORKSPACE_CRN}
      # The CipherStash client access key for making requests for encryption keys
      - CS_CLIENT_ACCESS_KEY=${CS_CLIENT_ACCESS_KEY}
      # The CipherStash dataset ID for generating and retrieving encryption keys
      - CS_DEFAULT_KEYSET_ID=${CS_DEFAULT_KEYSET_ID}
      # The CipherStash client ID used to programmatically access a dataset
      - CS_CLIENT_ID=${CS_CLIENT_ID}
      # The CipherStash client key used to programmatically access a dataset
      - CS_CLIENT_KEY=${CS_CLIENT_KEY}
      # Toggle Prometheus exporter for CipherStash Proxy operations
      - CS_PROMETHEUS__ENABLED=${CS_PROMETHEUS__ENABLED:-true}
```


For a fully-working example, go to [`docker-compose.yml`](../../docker-compose.yml).
Follow the steps in [Getting started](../README.md#getting-started) to see it in action.

Once you have set up a `docker-compose.yml`, start the Proxy container:

```bash
docker compose up
```

Connect your PostgreSQL client to Proxy on TCP 6432.
Point [Prometheus to scrape metrics](../reference/index.md#prometheus-metrics) on TCP 9930.

## Configuring Proxy

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

See [Proxy config options](../reference/index.md#proxy-config-options) for all the available options.

### Configuring Proxy with environment variables

If you are configuring Proxy with environment variables, these are the minimum environment variables required to run Proxy:

```bash
CS_DATABASE__NAME
CS_DATABASE__USERNAME
CS_DATABASE__PASSWORD
CS_WORKSPACE_CRN
CS_CLIENT_ACCESS_KEY
CS_DEFAULT_KEYSET_ID
CS_CLIENT_ID
CS_CLIENT_KEY
```

Read the full list of environment variables and what they do in the [reference documentation](../reference/index.md#proxy-config-options).

### Configuring Proxy with a TOML file

If you are configuring Proxy with a `cipherstash-proxy.toml` file, these are the minimum values required to run Proxy:

```toml
[database]
name = "cipherstash"
username = "cipherstash"
password = "password"

[auth]
workspace_crn = "cipherstash-workspace-crn"
client_access_key = "cipherstash-client-access-key"

[encrypt]
default_keyset_id = "cipherstash-default-keyset-id"
client_id = "cipherstash-client-id"
client_key = "cipherstash-client-key"
```

Read the full list of configuration options and what they do in the [reference documentation](../reference/index.md#proxy-config-options).

## Running Proxy locally

To run CipherStash Proxy locally for development:

```bash
# Install prerequisites
mise trust --yes && mise install

# Start PostgreSQL and install EQL
mise run postgres:up --extra-args "--detach --wait"
mise run postgres:setup

# Run Proxy as a local process
mise run proxy
```

Alternatively, run Proxy in a container:

```bash
mise run proxy:up --extra-args "--detach --wait"
```

See [Configuring Proxy](#configuring-proxy) for required environment variables and configuration options.

## Setting up the database schema

Under the hood, Proxy uses [CipherStash Encrypt Query Language](https://github.com/cipherstash/encrypt-query-language/) to index and search encrypted data.

When you start the Proxy container, you can install EQL by setting the `CS_DATABASE__INSTALL_EQL` environment variable:

```bash
CS_DATABASE__INSTALL_EQL=true
```

This will install the version of EQL bundled with the Proxy container.
The version of EQL bundled with the Proxy container is tested to work with that version of Proxy.

If you are following the [getting started](../README.md#getting-started) guide above, EQL is automatically installed for you.
You can also install EQL by running [the installation script](https://github.com/cipherstash/encrypt-query-language/releases) as a database migration in your application.

Once you have installed EQL, you can see what version is installed by querying the database:

```sql
SELECT eql_v3.version();
```

This will output the version of EQL installed.

### Creating columns with the right types

In your existing PostgreSQL database, you store your data in tables and columns.
Those columns have types like `integer`, `text`, `timestamp`, and `boolean`.
When storing encrypted data in PostgreSQL with Proxy, you use one of EQL's **encrypted domain types**, which are [provided by EQL](#setting-up-the-database-schema).

In EQL v3 these domain types are **self-configuring**: the type you choose for a column both marks it as encrypted *and* declares which searches it supports. This replaces EQL v2's model of a single opaque `eql_v2_encrypted` container type plus a separate `eql_v2.add_search_config` call per index — there is no separate index-configuration step, and no `eql_v2_configuration` table.

Domain types follow the naming pattern `eql_v3_<token>_<capability>`:

- **Storage only** — `eql_v3_text`, `eql_v3_integer`, `eql_v3_bigint`, `eql_v3_date`, `eql_v3_boolean`, and so on store an encrypted value that can be read back but not searched. (`boolean` is always storage-only: a two-value column would leak its distribution under any index.)
- **Ordering and range** — the `_ord` suffix (e.g. `eql_v3_integer_ord`, `eql_v3_date_ord`) adds ordering (`ORDER BY`) and range comparisons (`<`, `<=`, `>`, `>=`), and also supports equality (`=`). This is the recommended default and uses CLLW-OPE ordering.
- **Ordering and range via ORE** — the `_ord_ore` suffix (e.g. `eql_v3_integer_ord_ore`) is an alternative ordering scheme backed by block-ORE. Choose `_ord` or `_ord_ore` for a column, not both.
- **Full text search** — for `text`, `eql_v3_text_search` bundles equality, ordering, and fuzzy `LIKE`/`ILIKE` match in one type; `eql_v3_text_search_ore` is the ORE-backed variant, and `eql_v3_text_ord_ope` provides OPE ordering.
- **Encrypted JSON** — `eql_v3_json_search` stores encrypted JSON with SteVec containment (`@>`, `<@`) and path (`->`, `->>`) search. See [Searchable JSON](../reference/searchable-json.md).

Create a `users` table with an encrypted, fully-searchable `email` column:

```sql
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    email eql_v3_text_search
)
```

This creates a `users` table with two columns:

 - `id`, an autoincrementing integer column that is the primary key for the record
 - `email`, an encrypted `text` column that supports equality (`=`), ordering, and fuzzy `LIKE`/`ILIKE` matching — because it uses the `eql_v3_text_search` domain type

There are important differences between the plaintext columns you've traditionally used in PostgreSQL and encrypted columns with CipherStash Proxy:

- **Plaintext columns can be searched if they don't have an index**, albeit with the performance cost of a full table scan.
- **An encrypted column can only be searched in the ways its domain type allows.** Choose the domain type up front to match the queries you need: `eql_v3_text` if you only store and retrieve the value, `eql_v3_text_search` if you also need to compare and match it.

If you only needed equality on `email` — for example a lookup by exact address — you could store it as a scalar ordering type such as `eql_v3_text_ord_ope`, or use `eql_v3_text_search` when you also want partial matches with `LIKE`.

`_ord` (CLLW-OPE) produces ciphertexts that sort under PostgreSQL's native byte ordering, which makes ordering and range scans cheaper, but as an order-preserving scheme it reveals more about the relative order of stored values than the block-ORE `_ord_ore` variant does. Choose based on your performance and threat-model requirements; see the [EQL `INDEX` documentation](https://github.com/cipherstash/encrypt-query-language/blob/main/docs/reference/INDEX.md) for the full tradeoffs.


> [!IMPORTANT]
> The searches an encrypted column supports are fixed by its domain type. To change them you change the column's type (e.g. `ALTER TABLE users ALTER COLUMN email TYPE eql_v3_text_search`), and any data already stored must be re-encrypted under the new type — `SELECT` it out of the column and `UPDATE` it back — before the new capabilities apply to it.

To learn how to use encrypted indexes for other encrypted data types like `text`, `int`, `boolean`, `date`, and `jsonb`, see the [EQL documentation](https://github.com/cipherstash/encrypt-query-language/blob/main/docs/reference/INDEX.md).

When deploying CipherStash Proxy into production environments with real data, we recommend that you apply these database schema changes with the normal tools and process you use for making changes to your database schema.

To see more examples of how to modify your database schema, check out [the example schema](../sql/schema-example.sql) from [Getting started](#getting-started).

## Encrypting data in an existing database

CipherStash Proxy includes an `encrypt` tool – a CLI application to encrypt existing data, or to apply index changes after changes to the encryption configuration of a protected database.
See the [`encrypt` tool guide](../reference/encrypt-tool.md) for info about using the `encrypt` tool.

---

### Didn't find what you wanted?

[Click here to let us know what was missing from our docs.](https://github.com/cipherstash/proxy/issues/new?template=docs-feedback.yml&title=[Docs:]%20Feedback%20on%20how-to.md)
