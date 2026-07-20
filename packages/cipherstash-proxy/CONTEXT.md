# Proxy

Speaks the PostgreSQL wire protocol to client applications, hands their SQL to EQL Mapper
for type checking and rewriting, encrypts and decrypts column values through ZeroKMS, and
forwards everything to the real database. It owns everything about *connections* and
*ciphertext*; it owns nothing about type inference.

## Language

### Connections and statements

**Connection**:
One client application's TCP connection to Proxy, from startup to termination. All
per-client state hangs off it.
_Avoid_: session — PostgreSQL already owns that word for something else, and this
codebase has used it for both a connection and a single statement.

**Context**:
The mutable state belonging to one connection — its prepared statements, portals,
pending describes and executes, and keyset.

**Frontend** / **Backend**:
The two halves of the proxy pipeline by direction of travel: Frontend carries client to
database, Backend carries database to client. These are *not* the PostgreSQL protocol's
frontend/backend roles, and Backend is not the database.

**Statement**:
A type-analysed SQL statement — its param columns, projection columns, literal columns
and PostgreSQL param type OIDs. The parsed AST and EQL Mapper's `TypeCheckedStatement`
are different things; do not call either of them a Statement here.

**Portal**:
A prepared statement bound to parameter values, ready to execute. Either `Encrypted`,
carrying the analysed statement, or `Passthrough` when nothing in it is encrypted.

**Statement metrics scope**:
The measurement window around a single statement — opened at Parse or Query, closed when
the statement completes. Many of these occur per connection.
_Avoid_: session (`start_session`, `SessionId` and the
`..._statements_session_duration_seconds` metric all use this sense and are misnamed).

**Passthrough**:
Traffic forwarded without encryption or rewriting. Three independent conditions produce
it and they are not interchangeable: no encrypt config is loaded, mapping is disabled by
configuration, or `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING` was issued on this connection.
Name the condition rather than saying "passthrough" alone.

### Encryption

**Column**:
An encrypted column as this crate sees it — its `Identifier`, its column config, its
resolved PostgreSQL type, and the EQL term shape it takes in the current statement.
_Avoid_: confusing with EQL Mapper's schema/projection columns, or with `DataColumn`
(a value on the wire) and `RowDescriptionField` (a result descriptor).

**Identifier**:
The `table.column` pair that keys the encrypt config. EQL Mapper calls the same thing a
`TableColumn`.

**Encrypt credentials**:
The CipherStash client credentials Proxy authenticates to ZeroKMS with — client id,
client key, default keyset. Read from `[encrypt]` / `CS_ENCRYPT__*`.
_Avoid_: `EncryptConfig`, which currently names this *and* an unrelated struct.

**Column encrypt config**:
The per-column encryption configuration — which columns are encrypted and with which SEM
terms — read from the EQL config table in the *database* at runtime, and reloaded when
DDL is observed.
_Avoid_: `EncryptConfig`; also "encrypt schema", despite `ReloadCommand::EncryptSchema`
naming the reload of this thing.

**Client id**:
Ambiguous — always qualify. The **connection number** is the incrementing counter stamped
on log lines to correlate one connection's traffic. The **CipherStash client id** is the
credential UUID from `CS_ENCRYPT__CLIENT_ID`. They share a name and a field spelling and
are otherwise unrelated.

**Keyset**:
The ZeroKMS collection of keys a workspace encrypts against. Addressed by UUID or by
name, and selectable per connection via `SET CIPHERSTASH.KEYSET_ID` / `KEYSET_NAME`.

**Scoped cipher**:
A cipher bound to exactly one keyset. Cached per keyset, so switching keyset switches
cipher.

**Plaintext**:
A value in its typed, pre-encryption form. The boundary type between PostgreSQL wire
values and ZeroKMS.

**EQL ciphertext**:
An encrypted value in the shape stored in the database. What encryption produces and
decryption consumes.

**Store** / **Query** operation:
The two directions of encryption. Storing writes a value with all its SEM terms; querying
produces only the term needed to match. The statement's EQL term shape decides which.

### Control plane

**`SET CIPHERSTASH.*`**:
Proxy's in-band control API, intercepted rather than forwarded — `KEYSET_ID`,
`KEYSET_NAME`, `UNSAFE_DISABLE_MAPPING`. These are the supported spellings; log messages
that print `CIPHERSTASH.DISABLE_MAPPING` are wrong.

**Reload**:
Re-reading state from the database after observed DDL. Two independent things reload: the
database schema, and the column encrypt config.

## Note on `session`

This glossary bans `session` because the code uses it for two different things and
PostgreSQL uses it for a third. The identifiers `SessionId`, `start_session`,
`finish_session` and the Prometheus metric name all still carry it. Read them as
*statement metrics scope*; the doc comments on `Frontend` and `Backend` that say "session
context" mean *connection*.
