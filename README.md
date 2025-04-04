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

## Proxy V2 now available

[Read the announcement](https://cipherstash.com/blog/introducing-proxy)


CipherStash Proxy provides a transparent proxy to your existing Postgres database.

Proxy:
* Automatically encrypts and decrypts the columns you specify
* Supports most query types over encrypted values
* Runs in a Docker container
* Is written in Rust and uses a formal type system for SQL mapping
* Works with CipherStash ZeroKMS and offers up to 14x the performance of AWS KMS

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
  - [Encrypting data in an existing database](#encrypting-data-in-an-existing-database)
    - [Using the `encrypt` tool](#using-the-encrypt-tool)
    - [How the `encrypt` tool works](#how-the-encrypt-tool-works)
    - [Configuring the `encrypt` tool](#configuring-the-encrypt-tool)
    - [Example `encrypt` tool usage](#example-encrypt-tool-usage)
- [Reference](#reference)
  - [Proxy config options](#proxy-config-options)
  - [Prometheus metrics](#prometheus-metrics)
    - [Available metrics](#available-metrics)
  - [`encrypt` tool config options](#encrypt-tool-config-options)
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

# Install the CipherStash CLI
## macOS
brew install cipherstash/tap/stash
## Linux
## Download from https://github.com/cipherstash/cli-releases/releases/latest

# Setup your CipherStash configuration
stash setup --proxy
# ⬆️ this outputs creds to .env.proxy.docker

# Start the containers
docker compose up
```

This will start a PostgreSQL database on `localhost:5432`, and CipherStash Proxy on `localhost:6432`.
There's an example table called `users` that you can use to start inserting and querying encrypted data with.

> [!NOTE]
> In this example table we've chosen users' email, date of birth, and salary as examples of the kind of sensitive data that you might want to protect with encryption.

### Step 1: Insert and read some data <a id='getting-started-step-1'></a>

Now let's connect to the Proxy via `psql` and run some queries:

```bash
docker compose exec proxy psql postgres://cipherstash:3ncryp7@localhost:6432/cipherstash
```

This establishes an interactive session with the database, via CipherStash Proxy.

Now insert and read some data via Proxy:

```sql
INSERT INTO users (encrypted_email, encrypted_dob, encrypted_salary) VALUES ('alice@cipherstash.com', '1970-01-01', '100');

SELECT encrypted_email, encrypted_dob, encrypted_salary FROM users;
```

The `INSERT` statement inserts a record into the `users` table, and the `SELECT` statement reads the same record back.
Notice that it looks like nothing happened: the data in the `INSERT` was unencrypted, and the data in the `SELECT` is also unencrypted.

Now let's connect to the database directly via `psql` and see what the data actually looks like behind the scenes:

```bash
docker compose exec proxy psql postgres://cipherstash:3ncryp7@postgres:5432/cipherstash
```

This establishes an interactive session directly with the database (note the change of host to `postgres` and port to `5432`).

Now on this direct `psql` session, query the database directly:

```sql
SELECT encrypted_email, encrypted_dob, encrypted_salary FROM users;
```

You'll see the output is _much_ larger, because the `SELECT` returns the raw encrypted data.
The data is transparently encrypted and decrypted by Proxy in the `INSERT` and `SELECT` statements.

### Step 2: Update the data with a `WHERE` clause <a id='getting-started-step-2'></a>

In your `psql` connection to Proxy:

```bash
docker compose exec proxy psql postgres://cipherstash:3ncryp7@localhost:6432/cipherstash
```

Update the data we inserted in [Step 1](#getting-started-step-1), and read it back:

```sql
UPDATE users SET encrypted_dob = '1978-02-01' WHERE encrypted_email = 'alice@cipherstash.com';

SELECT encrypted_dob FROM users WHERE encrypted_email = 'alice@cipherstash.com';
```

In the `UPDATE` statement, the `=` comparison operation in the `WHERE` clause is evaluated against **encrypted** data.
In the `SELECT` statement, the `encrypted_email` value is transparently encrypted by Proxy, and compared in the database against the stored encrypted email value.
In the `SELECT` statement, the `SELECT` returns `1978-02-01`.

Back on the `psql` session connected directly to the database, verify the data is encrypted:

```sql
SELECT encrypted_email, encrypted_dob, encrypted_salary FROM users;
```

This `SELECT` shows the raw encrypted data — no plaintext to see.

### Step 3: Search encrypted data with a `WHERE` clause <a id='getting-started-step-3'></a>

In your `psql` connection to Proxy:

```bash
docker compose exec proxy psql postgres://cipherstash:3ncryp7@localhost:6432/cipherstash
```

Insert more records via Proxy, and query by email:

```sql
INSERT INTO users (encrypted_email, encrypted_dob, encrypted_salary) VALUES ('bob@cipherstash.com', '1991-03-06', '10');
INSERT INTO users (encrypted_email, encrypted_dob, encrypted_salary) VALUES ('carol@cipherstash.com', '2005-12-30', '1000');

SELECT encrypted_email, encrypted_dob, encrypted_salary FROM users WHERE encrypted_salary <= 100;
```

In the `INSERT` statement, the salary value is transparently encrypted by Proxy, and stored in the database in encrypted form.
In the `SELECT` statement, the `encrypted_salary` value is transparently encrypted and compared in the database against the stored encrypted salary value.
In the `SELECT` statement, the `<=` comparison operation in the `WHERE` clause is evaluated against **encrypted** data.
In the `SELECT` statement, the `SELECT` returns `alice` and `bob`, but not `carol`.

Query `users` by email:

```sql
SELECT encrypted_email, encrypted_dob, encrypted_salary FROM users WHERE encrypted_email LIKE 'alice';
```

The literal string `alice` is transparently encrypted by Proxy, and compared in the database against the stored encrypted date value.
The `LIKE` comparison operation is evaluated against **encrypted** data.
The `SELECT` will only return `alice`.


Finally, query `users` by date:

```sql
SELECT encrypted_email, encrypted_dob, encrypted_salary FROM users WHERE encrypted_dob > '2000-01-01' ;
```

The literal date `2000-01-01` is transparently encrypted by Proxy, and compared in the database against the stored encrypted date value.
The `>` comparison operation is evaluated against **encrypted** data.
The `SELECT` will only return `carol`.

Back on the `psql` session connected directly to the database, verify the data is encrypted:

```sql
SELECT encrypted_email, encrypted_dob, encrypted_salary FROM users;
```

This `SELECT` shows the raw encrypted data, no plaintext to see.

This demonstrates the power of CipherStash Proxy:

- Completely transparent encryption of sensitive data in PostgreSQL
- All data remains searchable, while being protected with non-deterministic AES-256-GCM encryption
- Zero changes required to your application's database queries

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
      - CS_WORKSPACE_ID=${CS_WORKSPACE_ID}
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
Follow the steps in [Getting started](#getting-started) to see it in action.

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
CS_WORKSPACE_ID
CS_CLIENT_ACCESS_KEY
CS_DEFAULT_KEYSET_ID
CS_CLIENT_ID
CS_CLIENT_KEY
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
default_keyset_id = "cipherstash-default-keyset-id"
client_id = "cipherstash-client-id"
client_key = "cipherstash-client-key"
```

Read the full list of configuration options and what they do in the [reference documentation](#proxy-config-options).

### Running Proxy locally

TODO: Add instructions for running Proxy locally

### Setting up the database schema

Under the hood, Proxy uses [CipherStash Encrypt Query Language](https://github.com/cipherstash/encrypt-query-language/) to index and search encrypted data.

When you start the Proxy container, you can install EQL by setting the `CS_DATABASE__INSTALL_EQL` environment variable:

```bash
CS_DATABASE__INSTALL_EQL=true
```

This will install the version of EQL bundled with the Proxy container.
The version of EQL bundled with the Proxy container is tested to work with that version of Proxy.

If you are following the [getting started](#getting-started) guide above, EQL is automatically installed for you.
You can also install EQL by running [the installation script](https://github.com/cipherstash/encrypt-query-language/releases) as a database migration in your application.

Once you have installed EQL, you can see what version is installed by querying the database:

```sql
SELECT cs_eql_version();
```

This will output the version of EQL installed.

#### Creating columns with the right types

In your existing PostgreSQL database, you store your data in tables and columns.
Those columns have types like `integer`, `text`, `timestamp`, and `boolean`.
When storing encrypted data in PostgreSQL with Proxy, you use a special column type called `cs_encrypted_v1`, which is [provided by EQL](#setting-up-the-database-schema).
`cs_encrypted_v1` is a container column type that can be used for any type of encrypted data you want to store or search, whether they are numbers (`int`, `small_int`, `big_int`), text (`text`), dates and times (`date`), or booleans (`boolean`).

Create a table with an encrypted column for `email`:

```sql
CREATE TABLE users (
    id SERIAL PRIMARY KEY,
    email cs_encrypted_v1
)
```

This creates a `users` table with two columns:

 - `id`, an autoincrementing integer column that is the primary key for the record
 - `email`, a `cs_encrypted_v1` column

There are important differences between the plaintext columns you've traditionally used in PostgreSQL and encrypted columns with CipherStash Proxy:

- **Plaintext columns can be searched if they don't have an index**, albeit with the performance cost of a full table scan.
- **Encrypted columns cannot be searched without an encrypted index**, and the encrypted indexes you define determine what kind of searches you can do on encrypted data.

In the previous step we created a table with an encrypted column, but without any encrypted indexes.

Now you can add an encrypted index for that encrypted column:

```sql
SELECT cs_add_index_v1(
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
SELECT cs_add_index_v1(
  'users',
  'email',
  'match',
  'text'
);

SELECT cs_add_index_v1(
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

To see more examples of how to modify your database schema, check out [the example schema](./docs/getting-started/schema-example.sql) from [Getting started](#getting-started).

## Encrypting data in an existing database

CipherStash Proxy includes an `encrypt` tool – a CLI application to encrypt existing data, or to apply index changes after changes to the encryption configuration of a protected database.

### Using the `encrypt` tool

Encrypt the `source` column data in `table` into the specified encrypted `target` column.
The `encrypt` tool connects to CipherStash Proxy using the `cipherstash.toml` configuration or `ENV` variables.

```
cipherstash-proxy encrypt [OPTIONS] --table <TABLE>  --columns <SOURCE_COLUMN=TARGET_COLUMN>...
```

### How the `encrypt` tool works

At a high-level, the process for encrypting a column in the database is:

1. Add a new encrypted destination column with the appropriate encryption configuration.
2. Using CipherStash Proxy to process:
  1. Select from the original plaintext column.
  2. Update the encrpted column to set the plaintext value.
3. Drop the original plaintext column.
4. Rename the encrypted column to the original plaintext column name.

The CipherStash Proxy `encrypt` tool automates the data process to encrypt one or more columns in a table.
Updates are executed in batches of 100 records (and the `batch_size` is configurable).
The process is idempotent and can be run repeatedly.

### Configuring the `encrypt` tool

The CipherStash Proxy `encrypt` tool reuses the CipherStash Proxy configuration for the Proxy connection details.
This configuration includes database server host, port, username, password, and database name.
See [`encrypt` tool config options](#encrypt-tool-config-options) for the available options.

### Example `encrypt` tool usage

Given a running instance of CipherStash Proxy and a `users` table with:
 - `id` – a primary key column
 - `email` – a source plaintext column
 - `encrypted_email` – a destination column configured to be encrypted text.

Encrypt `email` into `encrypted_email`:

```bash
cipherstash-proxy encrypt --table users --columns email=encrypted_email
```

Specify the primary key column:

```bash
cipherstash-proxy encrypt --table users --columns email=encrypted_email --primary-key user_id
```

Specify multiple primary key columns (compound primary key):

```bash
cipherstash-proxy encrypt --table users --columns email=encrypted_email --primary-key user_id tenant_id
```

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
# CipherStash Workspace ID
# Env: CS_WORKSPACE_ID
workspace_id = "cipherstash-workspace-id"

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

### `encrypt` tool config options

| Option                  | Description                                                    | Default         |
| ----------------------- | -------------------------------------------------------------- | --------------- |
| `-t`, `--table`         | Specifies the table to migrate                                 | None (Required) |
| `-c`, `--columns`       | List of columns to migrate (space-delimited key=value pairs)   | None (Required) |
| `-k`, `--primary-key`   | List of primary key columns (space-delimited)                  | `id`            |
| `-b`, `--batch-size`    | Number of records to process at once                           | `100`           |
| `-d`, `--dry-run`       | Runs without updating. Loads data but does not perform updates | None (Optional) |
| `-v`, `--verbose`       | Turn on additional logging output                              | None (Optional) |
| `-h`, `--help`          | Displays this help message                                     | -               |


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

### Require environment variables
```
CS_WORKSPACE_ID
CS_CLIENT_ACCESS_KEY
CS_DEFAULT_KEYSET_ID
CS_CLIENT_ID
CS_CLIENT_KEY
```


### Running the benchmark

```bash
cd tests/benchmark
mise run benchmark
```

Results are graphed in a file called `benchmark-{YmdHM}.png` where `YmdHM` is a generated timestamp.
Detailed results are generated in `csv` format and in the `results` directory.


## More info

### Developing for Proxy

Check out the [Proxy development guide](./DEVELOPMENT.md).



