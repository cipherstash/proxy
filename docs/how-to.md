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


For a fully-working example, go to [`docker-compose.yml`](./docker-compose.yml).
Follow the steps in [Getting started](../README.md#getting-started) to see it in action.

Once you have set up a `docker-compose.yml`, start the Proxy container:

```bash
docker compose up
```

Connect your PostgreSQL client to Proxy on TCP 6432.
Point [Prometheus to scrape metrics](reference.md#prometheus-metrics) on TCP 9930.

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

See [Proxy config options](reference.md#proxy-config-options) for all the available options.

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

Read the full list of environment variables and what they do in the [reference documentation](reference.md#proxy-config-options).

### Configuring Proxy with a TOML file

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
default_keyset_id = "cipherstash-default-keyset-id"
client_id = "cipherstash-client-id"
client_key = "cipherstash-client-key"
```

Read the full list of configuration options and what they do in the [reference documentation](reference.md#proxy-config-options).

## Running Proxy locally

TODO: Add instructions for running Proxy locally

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
SELECT eql_v2.version();
```

This will output the version of EQL installed.

### Creating columns with the right types

In your existing PostgreSQL database, you store your data in tables and columns.
Those columns have types like `integer`, `text`, `timestamp`, and `boolean`.
When storing encrypted data in PostgreSQL with Proxy, you use a special column type called `eql_v2_encrypted`, which is [provided by EQL](#setting-up-the-database-schema).
`eql_v2_encrypted` is a container column type that can be used for any type of encrypted data you want to store or search, whether they are numbers (`int`, `small_int`, `big_int`), text (`text`), dates and times (`date`. `timestamp`), or booleans (`boolean`).

Create a table with an encrypted column for `email`:

```sql
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    email eql_v2_encrypted
)
```

This creates a `users` table with two columns:

 - `id`, an autoincrementing integer column that is the primary key for the record
 - `email`, a `eql_v2_encrypted` column

There are important differences between the plaintext columns you've traditionally used in PostgreSQL and encrypted columns with CipherStash Proxy:

- **Plaintext columns can be searched if they don't have an index**, albeit with the performance cost of a full table scan.
- **Encrypted columns cannot be searched without an encrypted index**, and the encrypted indexes you define determine what kind of searches you can do on encrypted data.

In the previous step we created a table with an encrypted column, but without any encrypted indexes.

Now you can add an encrypted index for that encrypted column:

```sql
SELECT eql_v2.add_search_config(
  'users',
  'email',
  'unique',
  'text'
);
```

This statement adds a `unique` index for the `email` column in the `users` table, which has an underlying data type of `text`.

`unique` indexes are used to find records with columns with unique values, like with the `=` operator.

There are two other types of encrypted indexes you can use on `text` data:

```sql
SELECT eql_v2.add_search_config(
  'users',
  'email',
  'match',
  'text'
);

SELECT eql_v2.add_search_config(
  'users',
  'email',
  'ore',
  'text'
);
```

The first SQL statement adds a `match` index, which is used for partial matches with `LIKE`.
The second SQL statement adds an `ore` index, which is used for ordering with `ORDER BY`.

Now that the indexes has been added, you must activate them:

```sql
SELECT cs_encrypt_v1();
SELECT cs_activate_v1();
```

This loads and activates the encrypted indexes.

You must run the `cs_encrypt_v1()` and `cs_activate_v1()` functions after any modifications to the encrypted indexes.

> ![IMPORTANT]
> Adding, updating, or deleting encrypted indexes on columns that already contain encrypted data will not re-index that data. To use the new indexes, you must `SELECT` the data out of the column, and `UPDATE` it again.

To learn how to use encrypted indexes for other encrypted data types like `text`, `int`, `boolean`, `date`, and `jsonb`, see the [EQL documentation](https://github.com/cipherstash/encrypt-query-language/blob/main/docs/reference/INDEX.md).

When deploying CipherStash Proxy into production environments with real data, we recommend that you apply these database schema changes with the normal tools and process you use for making changes to your database schema.

To see more examples of how to modify your database schema, check out [the example schema](./getting-started/schema-example.sql) from [Getting started](#getting-started).

## Encrypting data in an existing database

CipherStash Proxy includes an `encrypt` tool – a CLI application to encrypt existing data, or to apply index changes after changes to the encryption configuration of a protected database.
See the [`encrypt` tool guide](encrypt-tool.md) for info about using the `encrypt` tool.

---

### Didn't find what you wanted?

[Click here to let us know what was missing from our docs.](https://github.com/cipherstash/proxy/issues/new?template=docs-feedback.yml&title=[Docs:]%20Feedback%20on%20how-to.md)
