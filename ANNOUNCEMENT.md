# CipherStash Proxy 3.1.0

CipherStash Proxy 3.1.0 makes Proxy safer behind connection poolers, closes several paths by which plaintext could reach PostgreSQL, and adds command-line flags for connecting to the database and requiring TLS.

This release refuses some statements that earlier versions forwarded, so read [Before you upgrade](#before-you-upgrade) first.
The full list of changes is in the [CHANGELOG](https://github.com/cipherstash/proxy/blob/main/CHANGELOG.md#310---2026-11-02).

## Before you upgrade

Each of these changes refuses something that 3.0 accepted.
Check that your applications and drivers do not rely on them.

- **Reserved EQL payload shapes**: on encrypted columns, JSON objects shaped like EQL storage or query payloads are now validated as EQL instead of being encrypted as plaintext, with no opt-out. Audit plaintext writes for these key combinations; [Invalid encrypted value](https://github.com/cipherstash/proxy/blob/main/docs/errors.md#encrypt-invalid-inbound-eql-payload) includes a `jsonb` scan predicate.
- **Settings commands Proxy cannot honour**: a simple query that changes a `CIPHERSTASH.*` setting and then needs encryption, unknown `CIPHERSTASH.*` setting names or values, `SET ALL`, and prepared statements made stale by a settings change are refused.
- **`SET LOCAL CIPHERSTASH.*`** now ends with the transaction instead of lasting for the connection. Use `SET` where a keyset must persist after `COMMIT`.
- **SQL `PREPARE` bound through the extended protocol** is refused with PostgreSQL's `26000` error. Prepare with Parse, or run with SQL `EXECUTE`.
- **Writes Proxy cannot resolve or type check** are refused while encrypted columns are configured, including `MERGE`, SQL `PREPARE` of a write, writes to views, foreign tables and schemas outside the loaded search path, and some bulk `UPDATE` and `INSERT` forms.
- **`COPY ... FROM` into encrypted columns** is refused. Load those columns with `INSERT`.
- **Default keyset**: when a connection selects no keyset, Proxy now uses `CS_DEFAULT_KEYSET_ID` rather than the account default. Verify the two are intended; values written under an unintended account default must be re-encrypted.
- **Building from source** requires Rust 1.89.0 or later.

The floating `cipherstash/proxy:3.0` Docker tag does not move to this release.
Pin `cipherstash/proxy:3.1` or `cipherstash/proxy:3.1.0` once you are ready.

## Highlights

### Safer behind connection poolers

`DISCARD ALL` and `RESET ALL` now return Proxy's per-connection keyset and mapping settings to their startup values, so a session-mode pooler no longer hands one client's keyset to the next.
`SET LOCAL CIPHERSTASH.KEYSET_ID`, `KEYSET_NAME` and `UNSAFE_DISABLE_MAPPING` give a setting that lasts for one transaction, which is the safe way to select a tenant's keyset behind a transaction-mode pooler.
[Connection poolers](https://github.com/cipherstash/proxy/blob/main/docs/reference/connection-poolers.md) documents which PgBouncer and PgCat versions and pool modes work with Proxy.

### Fewer paths for plaintext to reach PostgreSQL

Proxy now refuses writes it cannot type check, `COPY` into encrypted columns, and statements whose encryption results come back short, where it previously forwarded them as written.
Statements after `COPY ... FROM STDIN` in a simple query are now checked and encrypted like any other.

### Application-generated EQL values

Applications can send EQL v3 storage payloads and query operands they produced themselves.
Proxy authenticates each ciphertext, checks it names the destination column, and re-derives its search terms before forwarding it.

### New command-line flags

`--database-url` sets the database connection from one PostgreSQL connection string, and `--db-port` and `--db-password` join the existing `--db-*` flags.
`--tls` requires TLS from clients, and `--no-tls` forces a plaintext listener.
Run in a terminal, Proxy is now quiet by default and prints its listen address; pass `--debug` for debug logging.

### Fixes

This release also fixes query cancellation through Proxy, memory growth on long-lived connections that deallocate with SQL, schema-qualified table names, and the `--db-user` flag, which previously had no effect.
