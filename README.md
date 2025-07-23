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

CipherStash Proxy provides transparent, *searchable* encryption for your existing Postgres database.

CipherStash Proxy:
* Automatically encrypts and decrypts data with zero changes to SQL
* Supports queries over *encrypted* values:
   - equality
   - comparison
   - ordering
   - grouping
* Is written in Rust for high performance and strongly-typed mapping of SQL statements.
* Manages keys using CipherStash ZeroKMS, offering up to 14x the performance of AWS KMS

Behind the scenes, CipherStash Proxy uses the [Encrypt Query Language](https://github.com/cipherstash/encrypt-query-language/) to index and search encrypted data.

## Table of contents

- [Getting started](#getting-started)
  - [Step 1: Insert and read some data](#step-1-insert-and-read-some-data)
  - [Step 2: Update the data with a `WHERE` clause](#step-2-update-the-data-with-a-where-clause)
  - [Step 3: Search encrypted data with a `WHERE` clause](#step-3-search-encrypted-data-with-a-where-clause)
- [More info](#more-info)

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

### Step 1: Insert and read some data

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

### Step 2: Update the data with a `WHERE` clause

In your `psql` connection to Proxy:

```bash
docker compose exec proxy psql postgres://cipherstash:3ncryp7@localhost:6432/cipherstash
```

Update the data we inserted in [Step 1](#step-1-insert-and-read-some-data), and read it back:

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

### Step 3: Search encrypted data with a `WHERE` clause

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


## More info

Check out our [how-to guide](docs/how-to/index.md) for Proxy, or jump straight into the [reference guide](docs/reference/index.md).
For information on developing for Proxy, see the [Proxy development guide](./DEVELOPMENT.md).

---

### Didn't find what you wanted?

[Click here to let us know what was missing from our docs.](https://github.com/cipherstash/proxy/issues/new?template=docs-feedback.yml&title=[Docs:]%20Feedback%20on%20README.md)

