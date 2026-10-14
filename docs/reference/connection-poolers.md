# Connection poolers

This page covers running CipherStash Proxy with a PostgreSQL connection pooler: which topologies
work, which do not, and how to configure the ones that do.

## Proxy does not pool connections

Proxy opens one PostgreSQL connection for each client connection and closes it when the client
disconnects. There is no upstream connection pool inside Proxy. If your application needs
pooling, put a pooler in front of Proxy, behind it, or both, as described below.

Proxy keeps some state on each client connection, and a pooler decides which client uses which
connection. That interaction is what this page is about. The per-connection state is:

- the keyset selected with `SET CIPHERSTASH.KEYSET_ID` or `SET CIPHERSTASH.KEYSET_NAME`
- `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING`
- prepared statements, which Proxy maps when it sees `Parse` and uses when the statement is
  executed later by name

## Supported topologies

The topology tables describe the current matrix expectations, informed by the original October
2026 observations and inspection of the pinned pooler source. "Works" is the required outcome;
"Fails" or "Leaks" requires evidence of that specific incompatibility. The CI gate verifies
these outcomes and the supported Go suites. These tables do not establish support for every
driver or configuration.

The two topologies are:

- **Pooler in front of Proxy**: app → pooler → Proxy → PostgreSQL
- **Proxy in front of pooler**: app → Proxy → pooler → PostgreSQL

### Pooler in front of Proxy

| Pooler | Pool mode | CRUD (simple and extended protocol) | Named statement executed on another server connection | Session `SET CIPHERSTASH.KEYSET_*` inside `BEGIN`/`COMMIT` isolation | `SET LOCAL CIPHERSTASH.KEYSET_*` isolation | Session mapping `SET` inside `BEGIN`/`COMMIT` isolation |
|---|---|---|---|---|---|---|
| PgBouncer 1.19 | session | Works | n/a | Works | untested | Works |
| PgBouncer 1.19 | transaction | Works | **Fails** (1) | **Leaks** (2) | Works | **Leaks** (2) |
| PgBouncer 1.24, `max_prepared_statements = 0` | session | Works | n/a | Works | untested | Works |
| PgBouncer 1.24, `max_prepared_statements = 0` | transaction | Works | **Fails** (1) | **Leaks** (2) | Works | **Leaks** (2) |
| PgBouncer 1.24, `max_prepared_statements = 200` | session | Works | n/a | Works | untested | Works |
| PgBouncer 1.24, `max_prepared_statements = 200` | transaction | Works | Works | **Leaks** (2) | Works | **Leaks** (2) |
| PgCat 1.2, `prepared_statements_cache_size = 0` | session | Works | n/a | **Leaks** (3) | untested | **Leaks** (3) |
| PgCat 1.2, `prepared_statements_cache_size = 0` | transaction | Works | **Fails** (1) | **Leaks** (2) | Works | **Leaks** (2) |
| PgCat 1.2, `prepared_statements_cache_size = 500` | session | Works | n/a | **Leaks** (3) | untested | **Leaks** (3) |
| PgCat 1.2, `prepared_statements_cache_size = 500` | transaction | Works | Works | **Leaks** (2) | Works | **Leaks** (2) |
| none (app → Proxy) | n/a | Works | n/a | Works | Works | Works |

1. The pooler does not track prepared statements. `Bind`/`Execute` for a named statement reach
   a server connection that never received its `Parse`, and the client gets
   `prepared statement "…" does not exist`. Statement names can also collide between clients
   (`prepared statement "s0" already exists`). The same happens without Proxy.
2. A session-level `SET` inside an explicit transaction changes the Proxy connection beyond
   `COMMIT`. In transaction mode the pooler gives that connection to other clients, so they inherit
   the keyset or disabled mapping. **This is a cross-tenant leak**: a client that never selected a keyset could read and
   write another tenant's data. Use `SET LOCAL` (see below).
3. PgCat 1.2 only marks `SET` for cleanup when it observes the command outside an explicit
   transaction. A session-level `SET` between `BEGIN` and `COMMIT` is not marked, so it can survive
   client disconnection even in session mode. The main isolation cells exercise this untracked
   form deliberately. This behavior is visible in the pinned
   [PgCat command-completion handler](https://github.com/postgresml/pgcat/blob/4a7a6a8e7a78354b889002a4db118a8e2f2d6d79/src/server.rs#L1015).

PgCat's standalone `SET` follows a different path: with the fixture's default
`cleanup_server_connections = true`, it sends `RESET ROLE;RESET ALL;` before returning the backend
to the pool. Proxy now forwards these commands and updates its own settings after successful
execution. That fixes rejected cleanup; it cannot make PgCat clean a transaction it never marked.
The dedicated cleanup cells require the following outcomes:

| PgCat configuration | Pool mode | Standalone keyset `SET` cleanup | Standalone mapping `SET` cleanup |
|---|---|---|---|
| 1.2, cache size 0 | session | Works | Works |
| 1.2, cache size 0 | transaction | Works | Works |
| 1.2, cache size 500 | session | Works | Works |
| 1.2, cache size 500 | transaction | Works | Works |

The cleanup cells seed and verify data inside a transaction using `SET LOCAL`, then issue the
session-level `SET` outside it. In transaction mode PgCat can clear that standalone setting before
the issuing client's next query. Losing it at that boundary must not be mistaken for a failed
isolation setup or evidence of a cross-client leak.

"CRUD" is INSERT, SELECT (by id and by encrypted equality), UPDATE and DELETE over the simple
protocol; INSERT and SELECT with unnamed statements; and INSERT and SELECT with named statements
prepared once and executed several times inside one explicit transaction. This keeps Parse and
Bind on the same server connection. The separate named-statement switching check tests reuse
across transaction boundaries under contention; passing CRUD alone does not establish that support.
The CRUD probe retains its statements until disconnect. Explicitly closing named statements before
COMMIT is not covered: PgBouncer 1.24 with prepared-statement tracking can absorb the Close messages
and forward a standalone Sync, which the current pg-proto 0.12 pipeline rejects. CI observed the
connection closing in that sequence. This is another transaction-mode limitation, not a supported
workload.

### Proxy in front of pooler

| Pooler | Pool mode | CRUD (simple and extended protocol) | Named statement executed on another server connection |
|---|---|---|---|
| PgBouncer 1.19 | session, transaction | **Fails** (4) | **Fails** (4) |
| PgBouncer 1.24, `max_prepared_statements = 0` | session | Works | n/a |
| PgBouncer 1.24, `max_prepared_statements = 0` | transaction | Works | **Fails** (1) |
| PgBouncer 1.24, `max_prepared_statements = 200` | session | Works | n/a |
| PgBouncer 1.24, `max_prepared_statements = 200` | transaction | Works | Works |
| PgCat 1.2 (either cache size) | session, transaction | **Fails** (4) | **Fails** (4) |

4. These workloads cannot run because startup is rejected; they are not CRUD executions that
reached PostgreSQL. The current matrix records dedicated startup probes for these configurations.
Proxy opens its PostgreSQL connections with protocol version 3.2. PgBouncer before 1.23 and
   PgCat 1.2 reject that startup packet (`bad packet header`, `Unexpected startup code: 196610`)
   instead of negotiating down, so Proxy cannot connect to them at all. A separate probe with
   psql 18 showed PgBouncer 1.19, 1.21 and 1.22 reject a 3.2 startup and 1.23 and 1.24 accept it.

In this topology every client has its own Proxy connection, so Proxy's per-connection state is
never shared between clients. Keyset and mapping isolation are not at risk and were not run.

### Go pgx suite through each pooler

`POOLERS_GOLANG=informational mise run test:poolers` runs the Go integration suite
(`tests/integration/golang`, pgx v5, 474 tests and subtests, 32 in parallel) through each
pooler, using pools of 40. pgx caches named prepared statements and also exercises its
describe/exec modes, which use the unnamed statement across two round trips.

| Pooler | Pooler in front of Proxy, session | Pooler in front of Proxy, transaction | Proxy in front of pooler, session | Proxy in front of pooler, transaction |
|---|---|---|---|---|
| none (app → Proxy) | 474 passed | n/a | n/a | n/a |
| PgBouncer 1.19 | 474 passed | 92 failed (5) | cannot connect (4) | cannot connect (4) |
| PgBouncer 1.24, `max_prepared_statements = 0` | 474 passed | 85 failed (5) | 474 passed | 98 failed (5) |
| PgBouncer 1.24, `max_prepared_statements = 200` | 474 passed | 15 failed (6) | 474 passed | 78 failed (6) |
| PgCat 1.2, `prepared_statements_cache_size = 0` | 474 passed | 100 failed (5) | cannot connect (4) | cannot connect (4) |
| PgCat 1.2, `prepared_statements_cache_size = 500` | 474 passed | 1 failed (7) | cannot connect (4) | cannot connect (4) |

5. `prepared statement "stmtcache_…" does not exist` and `… already exists`: named statements
   without pooler support, as in (1).
6. `bind message supplies 1 parameters, but prepared statement "" requires 0`: pgx's
   describe/exec modes send `Parse` for the unnamed statement and `Bind` in a later round trip,
   which can land on another server connection. PgBouncer's `max_prepared_statements` only covers
   named statements. One test read a value of the wrong shape (`invalid length for bool: 33`),
   that is, another statement's result. Use pgx's default cached-statement mode, or session mode.
7. `TestPgxBatchPipelinesEncryptedDDLAndDependentInsert`: `prepared statement "PGCAT_16" does
   not exist` in a pipelined batch that mixes DDL with prepared statements.

The reported PgBouncer 1.24 errors also occur with Proxy in front of the pooler, where each
client has its own Proxy connection. This supports a prepared-statement compatibility explanation,
but does not rule out all Proxy defects in every failing driver case.

The default `mise run test:poolers` gates the complete Go suite on the eight direct/session-mode
configurations marked as passing above. It requires fresh, successful execution and the same test
identities as the direct control; empty, skipped, incomplete, or failed runs fail the task.
The full Go suite in transaction mode remains unsupported. Applicable transaction-mode behaviors
are checked by the Rust matrix, including named-statement switching and transaction-local isolation.

### Not covered

- **Supavisor**: not in the matrix. It needs its own metadata database and tenant registration
  through its API, which the test harness does not set up yet.
- **PgBouncer 1.20 to 1.23 in front of Proxy**: not run. Only their response to a protocol 3.2
  startup was probed.
- **Clients**: the matrix uses `tokio-postgres`, and the supported Go gate uses pgx. node-postgres
  and ORMs are not covered.
- **TLS between the pooler and Proxy**: not covered; the matrix uses plaintext inside a Docker
  network.
- **`SET LOCAL` in session mode**: not run separately. The behaviour does not depend on the pool
  mode, and it passed on a direct connection.

## Recommendations

### Pooler in front of Proxy

- **Transaction mode with named prepared statements needs pooler support for them.** Use
  PgBouncer 1.21 or later with `max_prepared_statements` above zero (tested: 1.24), or PgCat
  with `prepared_statements_cache_size` above zero. Most drivers use named statements by default.
  Even then, a driver that splits the unnamed statement's `Parse` and `Bind` across round trips
  fails in transaction mode (see the Go results above). In session mode every CRUD check and the
  whole Go suite passed with every pooler in front of Proxy.
- **With a default keyset** (`CS_DEFAULT_KEYSET_ID` set), `SET CIPHERSTASH.KEYSET_*` is rejected,
  so keysets cannot leak. Do not use `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING` through a
  transaction-mode pooler, or through PgCat in any mode.
- **Multitenant, without a default keyset**:
  - Select the keyset inside each transaction with `SET LOCAL`:

    ```sql
    BEGIN;
    SET LOCAL CIPHERSTASH.KEYSET_ID = '2cace9db-3a2a-4b46-a184-ba412b3e0730';
    -- statements for this tenant
    COMMIT;
    ```

    Proxy forgets a `SET LOCAL` keyset when PostgreSQL reports the transaction finished, before
    the pooler can hand the connection to another client. The current implementation has protocol
    regression coverage and CI has verified transaction-mode cleanup across the matrix. The matrix
    does not establish session-mode `SET LOCAL` isolation.
    Settings change only after PostgreSQL confirms execution. Rollback restores session-level
    changes, rollback to a savepoint restores both scopes, and `COMMIT` on an aborted transaction
    is treated as rollback. Preparing a settings command has no effect.
  - A session-level `SET CIPHERSTASH.KEYSET_*` is only safe with PgBouncer in session mode, and
    only while PgBouncer's `server_reset_query` is `DISCARD ALL` (the default). Proxy clears its
    `SET CIPHERSTASH.*` values after PostgreSQL successfully executes `DISCARD ALL`.
  - Never use a session-level `SET CIPHERSTASH.KEYSET_*` behind a transaction-mode pooler or
    behind PgCat.

Send a settings change and dependent SQL as separate simple-query messages. A single simple-query
batch cannot safely encrypt dependent values before its settings command succeeds, so Proxy rejects
that combination. Extended-protocol execution waits for the relevant outcome before processing
dependent work. If a keyset change makes already encrypted literals or bound values stale, prepare
and bind again; mapping-mode changes can also invalidate prepared work.

Prepared statements whose keyset or mapping assumptions become stale fail with SQLSTATE
`26000`. Close, prepare, and bind may be pipelined to recover; the proxy restores retained
metadata when PostgreSQL rejects speculative replacements. JDBC can use this SQLSTATE for
cache recovery; pgx also invalidates cached statements on other execution errors. Automatic
retry remains subject to the driver's transaction rules.

Ordinary transaction and reset commands do not require settings round trips when their
execution cannot change the effective keyset or mapping mode. COPY input continues to flow
while the proxy waits for transaction outcomes.

### Proxy in front of pooler

- Use PgBouncer 1.23 or later. Earlier PgBouncer versions and PgCat 1.2 cannot accept Proxy's
  connections.
- For transaction mode, enable `max_prepared_statements`. Proxy forwards the application's named
  statements unchanged.

### Idle timeouts

Proxy closes a client connection that has been idle for `CS_DATABASE__CONNECTION_TIMEOUT`
milliseconds, 120000 (two minutes) by default. A pooler in front of Proxy holds idle server
connections for longer than that by default (PgBouncer `server_idle_timeout` 600 s, PgCat
`idle_timeout` 600000 ms). When the pooler reuses a connection Proxy has already closed:

- PgBouncer notices and opens a new connection.
- PgCat fails its health check, bans the server for `ban_time`, and returns `AllServersDown` to
  clients in the meantime. This was observed while building the matrix.

Set the pooler's idle timeout below Proxy's, or raise `CS_DATABASE__CONNECTION_TIMEOUT` (`0`
disables it). The matrix uses 60 seconds for both poolers.

## Testing

`mise run test:poolers` builds Proxy from source, starts PostgreSQL, two Proxies (with and without
a default keyset), each pooler, and one Proxy in front of each pooler, then runs the matrix in
`packages/cipherstash-proxy-integration/src/pooler`. It needs Docker and CipherStash credentials,
as described in [DEVELOPMENT.md](../../DEVELOPMENT.md).

The default gate builds current source and runs without filters. CI uses the same gate on PostgreSQL
17. Evidence is retained in `target/pooler-results`, including matrix results and Go JSON events.
Use `POOLERS_GOLANG=off` for local Rust-only diagnosis or `POOLERS_GOLANG=informational` for
explicitly non-gating Go exploration of unsupported configurations. Neither mode is accepted in CI.

Each cell is one test. There are 94 database cells, plus separate regression tests for the
matrix runner. Expected outcomes live in `expectation()` in
`packages/cipherstash-proxy-integration/src/pooler/mod.rs`. A known incompatibility passes its
test only when its specific cause is observed. Setup failures, unrelated errors, inconclusive
probes, and timeouts fail, including in unsupported configurations. Unexpected success also
fails so that the support table can be reviewed.

Isolation probes verify physical backend reuse and compare the actual plaintext or ciphertext
observed by the two clients. Missing rows, changed backend PIDs, unrelated errors, and cleanup
failures fail the probe. Expected leaks are followed by explicit cleanup so they cannot contaminate
later cells. Startup probes require explicit evidence of protocol 3.2 refusal.
PgBouncer must return its specific error response. Pinned PgCat 1.2 closes the socket instead,
so its probe requires two clean refusals, each bracketed by successful authenticated protocol 3.0
connections. A generic closed socket does not establish incompatibility. Configurations that
refuse startup have no CRUD coverage through Proxy in that topology.

Historical results above: Proxy built from the original matrix branch, PgBouncer 1.19.0 (Alpine 3.18) and 1.24.0
(Alpine 3.22), PgCat 1.2.0 (`ghcr.io/postgresml/pgcat:v1.2.0`), PostgreSQL 17, October 2026.

The current PgCat fixture builds official v1.2.0 source at commit
[`4a7a6a8`](https://github.com/postgresml/pgcat/tree/4a7a6a8e7a78354b889002a4db118a8e2f2d6d79)
with its committed Cargo lockfile. The Dockerfile verifies the source archive checksum; this
replaces the unavailable release image without changing the tested PgCat version.
