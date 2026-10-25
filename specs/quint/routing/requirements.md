# Statement routing — formal model contract

## Intent

For every statement a client sends, Proxy decides whether to map it (encrypt its values and rewrite it), forward it as the client wrote it, or refuse it.
The decision must never let a client's plaintext reach a column that is encrypted, except when the client has explicitly disabled mapping with `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING`.
Refusing is the default: a statement is forwarded only for a named reason.

This model checks that intent against the routing policy at the design level.
It does not execute Proxy's Rust code.

## Sources of truth

- `Frontend::route`, `Frontend::may_fall_back` and `Frontend::mapping_error` in `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs` on cipherstash/proxy#484 (refuse-by-default routing).
- The simple Query batch rule in `Frontend::query_handler` on #484: a fallback must not forward a batch that also holds a statement that needs mapping.
- `eql_mapper::requires_type_check` and `eql_mapper::writes_data` in `packages/eql-mapper/src/eql_mapper.rs`.
- `Context::begin_bind` and `Context::finish_parse` in `packages/cipherstash-proxy/src/postgresql/context/lifecycle.rs`: Bind reuses the Statement its Parse stored, and a Parse that was forwarded stores none.
- The Proxy glossary in `packages/cipherstash-proxy/CONTEXT.md`: effective schema, committed schema snapshot, schema publication, passthrough conditions.
- #486 (refuse writes to a table Proxy cannot resolve) and the batch DDL and temporary table work on #484 (temporary tables in the effective schema).

## Scope

In:

- One client connection, with transactions, simple Query batches of one or two statements, Parse, Bind/Execute, and Close.
- `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING`, as a session setting that has already executed successfully.
- `mapping_errors_enabled`, both values, fixed for a run.
- Columns that are native or encrypted in PostgreSQL, and columns whose table Proxy cannot resolve.
- Statements that read or write one column, and that type check, hit a coverage gap (the type check fails on valid SQL), or are a kind Proxy never type checks.
- Optionally, another connection committing DDL that changes a column between native and encrypted, followed by schema publication.
- PostgreSQL accepting or rejecting a forwarded write.

Out:

- Encryption itself, keysets and ZeroKMS: a mapped value is ciphertext.
- Reads leaking plaintext search terms in a `WHERE` clause; this model only tracks values written.
- The decrypted results a client sees.
- `SET` transaction and savepoint scope, `RESET`, `DISCARD ALL` and pooling.
- Connection-local DDL, temporary tables and the transaction schema overlay.
- `DEALLOCATE`, unnamed statements, portals as separate objects, Describe, Sync and pipelining.
- Configuration-disabled mapping and an empty encrypt config as whole-connection passthrough.
- Legacy EQL v2 columns (`UnmappableEncryptedColumn`).
- Values already stored in a column when DDL changes its type: converting existing data is the migration's job, not routing's.

## Actors and failure model

- **Client**: sends any statement in the model's finite universe, in any order.
- **Proxy**: routes each statement against its effective schema, which is pinned at `BEGIN` and is otherwise the latest published snapshot.
- **PostgreSQL**: applies a forwarded message atomically, and sees the newest committed column types at once.
- **Other connection** (optional): commits DDL that changes a column's type; Proxy publishes the reloaded snapshot in the same step.
- Messages are reliable and ordered. There are no crashes or timeouts.
- No fairness assumptions; only safety properties are checked.

## Assumptions

| ID | Assumption | Why it matters |
|---|---|---|
| A1 | A coverage gap is nondeterministic: any type-checked statement may hit one. | Over-approximates EQL Mapper's incompleteness instead of modelling SQL. |
| A2 | The encrypt config covers every encrypted column in the catalog, including one whose table name Proxy cannot resolve. | `has_encrypted_columns` gates the unresolvable-table refusal (#486). If an encrypted table were missing from the config, S2 would not protect it. |
| A3 | Statement kinds Proxy never type checks (`COPY ... FROM STDIN`, `DO`, `CALL`) carry no values for a column. Switched off in the `uncheckedWrites` instance. | `requires_type_check` forwards these as written. |
| A4 | PostgreSQL rejects a plaintext value written to an EQL domain-typed column. Switched off where S3 must rest on Proxy alone. | The domain's base type and CHECK constraint are the last line of defence. |
| A5 | A schema publication and the DDL that caused it are one step. | Publication completes before `ReadyForQuery` reaches the DDL's own client. |

## Properties

| ID | Property | Kind | Rationale | Status |
|---|---|---|---|---|
| S1 | Proxy never forwards plaintext for a column that its schema said was encrypted when it routed the statement, unless mapping was disabled. | Safety | Core promise of refuse-by-default routing. | Intended. Violated as built (F3); holds with the D2 candidate fix. |
| S2 | While encrypted columns are configured, Proxy never forwards plaintext for a column whose table it cannot resolve. | Safety | Unresolvable-table refusal (#486). | Intended and implemented on #484. |
| S3 | No write stores plaintext in a column that is encrypted at the time of the write, except a write made with mapping disabled. Values already in a column when DDL later makes it encrypted are out of scope. | Safety | The end-to-end promise. | Holds with a stable schema on Proxy alone once D2 is fixed; with concurrent DDL it rests on A4 (F1). |
| S4 | Proxy never forwards plaintext for a column that its schema says is encrypted at the moment it sends the write. | Safety | Stronger than S1: also covers routes that went stale. | Not required for now (D1 decided). Violated by F1. |

## Findings

- **F1: stale routes under concurrent DDL.** Proxy routes a prepared statement once, at Parse, and a transaction pins the schema at `BEGIN`. If another connection makes a column encrypted in between, Proxy forwards plaintext for it. Today PostgreSQL rejects that write (A4), so S3 holds in production, but it holds because of PostgreSQL rather than Proxy.
- **F2: kinds Proxy never type checks.** If A3 is false, a `COPY ... FROM STDIN`, `DO` or `CALL` that writes to an encrypted column is forwarded with plaintext. The model cannot tell how likely that is; it shows that nothing in routing prevents it.
- **F3: Parse under `UNSAFE_DISABLE_MAPPING`, Bind after it is turned off.** The Parse is forwarded and stores no Statement, so a later Bind with mapping enabled again forwards plaintext for an encrypted column. Bind only checks the setting in force at Bind to pass a statement through, not the other way round. The same code shape is on `main` (`parse_handler`'s early return and `bind_handler`'s `knowledge.statement == None` path). PostgreSQL's domain check (A4) is again what rejects the value.

None of these is confirmed as a reachable plaintext leak in production: each needs a test against Proxy and PostgreSQL first, written as a failing test before any fix.

## Open decisions

- **D1** (decided 2026-10-25): For now, PostgreSQL's type and domain checks (A4) are the accepted guard against routes made stale by concurrent DDL. Proxy will not route again at Bind or invalidate prepared routes, so S4 is not a requirement. An integration test that proves PostgreSQL rejects the stale write is planned. Revisit if A4 stops holding, for example for a domain whose CHECK accepts arbitrary JSON.
- **D2**: What should a Bind do with a statement whose Parse was passed through only because mapping was disabled, once mapping is enabled again? The model's candidate (`BIND_RECHECKS_MAPPING`) refuses it, which makes S1 and S3 hold. Routing it at Bind is the alternative.
- **D3**: Should Proxy refuse, or type check, writes by statement kinds it does not type check today (A3, F2)?
