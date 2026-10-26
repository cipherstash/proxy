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
- `eql_mapper::requires_type_check`, `eql_mapper::writes_data`, `eql_mapper::copies_into_table` and `eql_mapper::requires_schema` in `packages/eql-mapper/src/eql_mapper.rs`.
- `check_copy_into` in `frontend.rs` (#488): while encrypted columns are configured, `COPY ... FROM` into an encrypted column, or into a table Proxy cannot resolve, is refused.
- `Context::begin_bind` and `Context::finish_parse` in `packages/cipherstash-proxy/src/postgresql/context/lifecycle.rs`: Bind reuses the Statement its Parse stored, and a Parse that was forwarded stores none.
- `SettingsProtocol::bind` and `SettingsProtocol::execute` in `packages/cipherstash-proxy/src/postgresql/context/settings_protocol.rs`: for a statement `requires_schema` accepts, Bind is refused with `MappingError::PreparedSettingsChanged` when the mapping setting differs from the one at Parse, and Execute when it differs from the one at Bind.
- The Proxy glossary in `packages/cipherstash-proxy/CONTEXT.md`: effective schema, committed schema snapshot, schema publication, passthrough conditions.
- #486 (refuse writes to a table Proxy cannot resolve) and the batch DDL and temporary table work on #484 (temporary tables in the effective schema).

## Source-file correspondence

Each model element stands for the Rust code below, as merged to `main` with #484.
When a change touches one of these functions, check the model element beside it: update the model and rerun `./check.sh all` in the same change, or say in the PR why the model is unaffected.

| Model element (`routing.qnt`) | Rust code |
|---|---|
| `route`, `mayFallBack`, `copyRefused` | `Frontend::route`, `Frontend::may_fall_back`, `Frontend::mapping_error`, `check_copy_into` in `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`; `Error::is_coverage_gap` in `packages/cipherstash-proxy/src/error.rs`; `Context::has_encrypted_columns`, `Context::unsafe_disable_mapping` in `packages/cipherstash-proxy/src/postgresql/context/mod.rs`; `SchemaMiddleware::schema_has_encrypted_columns` in `packages/cipherstash-proxy/src/proxy/schema/middleware.rs` |
| `Kind`, `Op`, `requiresSchema` | `eql_mapper::requires_type_check`, `eql_mapper::writes_data`, `eql_mapper::copies_into_table`, `eql_mapper::requires_schema` in `packages/eql-mapper/src/eql_mapper.rs` |
| `simpleQuery`, `batchRefused` | `Frontend::query_handler` in `frontend.rs`; `Context::prepare_schema_for_statement`, `Context::batch_table_resolvers`, `Context::register_query` in `context/mod.rs` |
| `parse` | `Frontend::parse_handler` in `frontend.rs`; `Context::begin_parse`, `Context::prepare_parse`, `Context::finish_parse`, `Context::refuse_parse` in `packages/cipherstash-proxy/src/postgresql/context/lifecycle.rs`; `Context::prepare_settings_statement`, `Context::restore_failed_preparation` in `context/mod.rs`; `SettingsProtocol::prepare`, `SettingsProtocol::start_prepare`, `SettingsProtocol::failed` in `packages/cipherstash-proxy/src/postgresql/context/settings_protocol.rs` |
| `bindExecute`, `settingsChanged` | `Frontend::bind_handler`, `Frontend::intercept` (Execute) in `frontend.rs`; `Context::begin_bind`, `Context::execute` in `lifecycle.rs`; `Context::bind_settings_statement`, `Context::execute_settings_portal`, `Context::is_prepared` in `context/mod.rs`; `SettingsProtocol::bind`, `SettingsProtocol::execute`, `EffectiveSettings::refusal` in `settings_protocol.rs` |
| `close` | `Context::close` in `lifecycle.rs` |
| `begin`, `endTx`, `effective` | `SchemaMiddleware::before_statement`, `SchemaMiddleware::ready_for_query`, `SchemaMiddleware::adopt_latest`, `SchemaMiddleware::execute`, `SchemaMiddleware::simple_query` in `middleware.rs`; `Context::became_ready` in `lifecycle.rs` |
| `concurrentDdl`, `published`, `load` | `SchemaManager::reload`, `init_reloader`, `load_snapshot`, `build_snapshot` in `packages/cipherstash-proxy/src/proxy/schema/manager.rs`; `SCHEMA_QUERY` in `packages/cipherstash-proxy/src/proxy/sql/select_table_schemas.sql`; `Context::reload_schema`, `Context::publish_schema_if_changed` in `context/mod.rs` |
| `unresolvable`, `VUnknown` | `UnresolvedTable` handling in `Frontend::mapping_error` (`MappingError::UnresolvableTable`, `MappingError::AmbiguousTable`); the `check_copy_into` refusals |
| `setUnsafe`, `conn.unsafe` | `ConnectionSettings::unsafe_disable_mapping`, `ConnectionSettings::set_unsafe_disable_mapping`, `ConnectionSettings::command_succeeded` in `packages/cipherstash-proxy/src/postgresql/context/settings.rs` |

## Scope

In:

- One client connection, with transactions, simple Query batches of one or two statements, Parse, Bind/Execute, and Close. A two-statement batch holds two different type-checked statements; the batch rule concerns only those, and other kinds are sent alone to keep TLC's state space small.
- `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING`, as a session setting that has already executed successfully.
- `mapping_errors_enabled`, both values, fixed for a run.
- Columns that are native or encrypted in PostgreSQL, and columns whose table Proxy cannot resolve.
- Statements that read or write one column, and that type check, hit a coverage gap (the type check fails on valid SQL), are a `COPY ... FROM`, nest a write in a statement Proxy does not type check, or are opaque to Proxy (`DO`, `CALL`, SQL `EXECUTE`).
- Optionally, another connection committing DDL that changes a column between native and encrypted, followed by schema publication.
- PostgreSQL accepting or rejecting a forwarded write.

Out:

- Encryption itself, keysets and ZeroKMS: a mapped value is ciphertext.
- Reads leaking plaintext search terms in a `WHERE` clause; this model only tracks values written.
- The decrypted results a client sees.
- `SET` transaction and savepoint scope, `RESET`, `DISCARD ALL` and pooling.
- Connection-local DDL, temporary tables and the transaction schema overlay.
- `DEALLOCATE`, unnamed statements, portals as separate objects, Describe, Sync and pipelining. Bind and Execute are one step (D4).
- Configuration-disabled mapping. An empty encrypt config is in scope as `encConfigured = false`.
- Refusals that do not affect what is written, which Proxy makes and the model does not: `CopyWithEncryptedStatement`, `DependentStatementAfterSettings`, `DependentStatementAfterDdl`, `UnmodelledDdl`, `SchemaCouldNotBeLoaded`, type-check errors that are not coverage gaps, and messages suppressed after a failure in an extended batch. Leaving them out over-approximates what Proxy forwards.
- Legacy EQL v2 columns (`UnmappableEncryptedColumn`).
- Values already stored in a column when DDL changes its type: converting existing data is the migration's job, not routing's.

## Actors and failure model

- **Client**: sends any statement in the model's finite universe, in any order.
- **Proxy**: routes each statement against its effective schema, which is pinned at `BEGIN` and is otherwise the latest published snapshot. Proxy pins it from any statement until `ReadyForQuery(Idle)`, so implicit transactions pin it too; the model's `begin` covers those states.
- **PostgreSQL**: applies a forwarded message atomically, and sees the newest committed column types at once.
- **Other connection** (optional): commits DDL that changes a column's type, and Proxy publishes the reloaded snapshot. The model makes these one step (A5).
- Messages are reliable and ordered. There are no crashes or timeouts.
- No fairness assumptions; only safety properties are checked.

## Assumptions

| ID | Assumption | Why it matters |
|---|---|---|
| A1 | A coverage gap is nondeterministic: any type-checked statement may hit one. | Over-approximates EQL Mapper's incompleteness instead of modelling SQL. |
| A2 | The encrypt config covers every encrypted column in the catalog, including one whose table name Proxy cannot resolve. | `has_encrypted_columns` gates the unresolvable-table refusal (#486). If an encrypted table were missing from the config, S2 would not protect it. This holds for a table name in two schemas, since both are loaded. It does not hold for a table outside the loader's `current_schemas`, which is neither loaded nor configured; that case is under review separately. |
| A3 | Statements Proxy cannot inspect (`DO`, `CALL`, SQL `EXECUTE`) carry no values for a column. Switched off in the `uncheckedWrites` instance. | `route` forwards these as written. `COPY ... FROM` and nested writes are not covered by A3: the model routes them as Proxy does. Whether SQL `PREPARE` under `UNSAFE_DISABLE_MAPPING` followed by `EXECUTE` breaks A3 is under review separately. |
| A4 | PostgreSQL rejects a plaintext value written to an EQL domain-typed column. Switched off where S3 must rest on Proxy alone. | The domain's base type and CHECK constraint are the last line of defence. Tested through Proxy for F1 in `packages/cipherstash-proxy-integration/src/postgres_rejects_plaintext.rs` (#489). Since #488, Proxy itself refuses `COPY ... FROM` into an encrypted column. |
| A5 | A schema publication and the DDL that caused it are one step. | An over-approximation of how soon Proxy catches up. DDL through this Proxy commits first and is published before `ReadyForQuery` reaches its client; DDL from anywhere else is published by the periodic reload, 60 s by default. In between, Proxy routes against the old snapshot even outside a transaction, which widens F1. S1, S2 and S4 compare against Proxy's own view, so they are unaffected; S3 already rests on A4 under concurrent DDL. |
| A6 | A nested write takes no parameters: its values are in the SQL text of its Parse. | PostgreSQL accepts no parameters in a `COPY`. The settings check leaves nested writes out (`requires_schema`), so a nested write prepared with mapping disabled and bound after mapping is enabled is forwarded; under A6 its values crossed Proxy with mapping disabled, so it is not a violation of S1. |

## Properties

| ID | Property | Kind | Rationale | Status |
|---|---|---|---|---|
| S1 | Proxy never forwards plaintext for a column that its schema said was encrypted when it routed the statement, unless mapping was disabled. | Safety | Core promise of refuse-by-default routing. | Intended and holds as built. |
| S2 | While encrypted columns are configured, Proxy never forwards plaintext for a column whose table it cannot resolve. | Safety | Unresolvable-table refusal (#486). | Intended and implemented on #484. |
| S3 | No write stores plaintext in a column that is encrypted at the time of the write, except a write made with mapping disabled. Values already in a column when DDL later makes it encrypted are out of scope. | Safety | The end-to-end promise. | Holds as built with a stable schema on Proxy alone; with concurrent DDL it rests on A4 (F1). |
| S4 | Proxy never forwards plaintext for a column that its schema says is encrypted at the moment it sends the write. | Safety | Stronger than S1: also covers routes that went stale. | Not required for now (D1 decided). Violated by F1. |

## Findings

- **F1: stale routes under concurrent DDL.** Proxy routes a prepared statement once, at Parse, and a transaction pins the schema at `BEGIN`. If another connection makes a column encrypted in between, Proxy forwards plaintext for it. Until the next publication (A5), even a simple Query outside a transaction routes against the old snapshot. Today PostgreSQL rejects that write (A4), so S3 holds in production, but it holds because of PostgreSQL rather than Proxy. Without the domain check, S4 fails as well as S3.
- **F2: statements Proxy cannot inspect.** If A3 is false, a `DO`, `CALL` or SQL `EXECUTE` that writes to an encrypted column is forwarded with plaintext. The model cannot tell how likely that is; it shows that nothing in routing prevents it. In `uncheckedWrites`, S2, S3 and S4 fail as well as S1. Since #488, `COPY ... FROM` is refused instead, so it is no longer part of F2.
- **F3 (retracted 2026-10-25).** An earlier version of this model left out the Bind settings check in `SettingsProtocol::bind`. It reported that a statement prepared under `UNSAFE_DISABLE_MAPPING` and bound after mapping is enabled again forwards plaintext. Proxy built from `main` refuses that Bind with `PreparedSettingsChanged`, as an integration test in #489 shows. The model now includes the check, and mutation M4 shows S1 depends on it. The lesson: a design model reviewed against the code by hand can miss a guard, and a Quint Connect adapter would have caught this one.

Neither F1 nor F2 is confirmed as a reachable plaintext leak in production: each needs a test against Proxy and PostgreSQL first, written as a failing test before any fix.

## Decisions

- **D1** (decided 2026-10-25): For now, PostgreSQL's type and domain checks (A4) are the accepted guard against routes made stale by concurrent DDL. Proxy will not route again at Bind or invalidate prepared routes, so S4 is not a requirement. Integration tests in #489 prove that PostgreSQL rejects the stale write. Revisit if A4 stops holding, for example for a domain whose CHECK accepts arbitrary JSON.
- **D3** (decided 2026-10-25): Proxy will refuse `COPY ... FROM` into encrypted columns on `main`, as release/2.2 does (#483). `DO` and `CALL` stay forwarded, as a documented limit: their values are inside server-side code that Proxy cannot inspect. Done in #488, and modelled as `CopyFrom`.
- **D4** (2026-10-26): Bind and Execute stay one step. `SettingsProtocol::execute` refuses an Execute whose mapping setting changed since Bind, so splitting them would add only refused states. The cost is that the model cannot show S1 depends on the Execute check; the correspondence table lists it so that a change to it is reviewed against the model.
