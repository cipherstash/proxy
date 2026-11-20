# Audit context dossier: packages/cipherstash-proxy (Backend / driver / settings pass)

This is the second context-building pass over `packages/cipherstash-proxy`.
The first pass (Frontend focus) is at `audit-context-frontend/DOSSIER.md`.
This pass covers the database-to-client half, the per-connection driver loop, the settings and schema outcome machinery, and the read-path decryption chain.
It is a system model: structure, invariants, and assumptions.
It names no vulnerabilities, proposes no fixes, and assigns no severity.
An assumption that nothing enforces is recorded as an unenforced assumption and nothing more.

Per-function prose is under `audit-context/functions/`.
Records are referred to as R01 to R12:

| Id | Function | File | Prose |
| --- | --- | --- | --- |
| R01 | `Backend::intercept_backend` | `postgresql/middleware/backend.rs` L156-L377 | `functions/01-Backend-intercept_backend.md` |
| R02 | `Backend::flush_encrypted_rows` | `postgresql/middleware/backend.rs` L473-L567 | `functions/02-Backend-flush_encrypted_rows.md` |
| R03 | `SettingsProtocol::failed` | `postgresql/context/settings_protocol.rs` L331-L432 | `functions/03-SettingsProtocol-failed.md` |
| R04 | `ConnectionSettings::command_succeeded` | `postgresql/context/settings.rs` L317-L366 | `functions/04-ConnectionSettings-command_succeeded.md` |
| R05 | `Context::settings_execution_succeeded` | `postgresql/context/mod.rs` L1568-L1606 | `functions/05-Context-settings_execution_succeeded.md` |
| R06 | `SettingsProtocol::succeeded` | `postgresql/context/settings_protocol.rs` L287-L325 | `functions/06-SettingsProtocol-succeeded.md` |
| R07 | `postgresql::handler` (forward loop, capacity retry, outcome drain) | `postgresql/driver.rs` L85-L342 | `functions/07-postgresql-handler-forward-loop-capacity-retry-outcome-drain.md` |
| R08 | `SchemaMiddleware::execution_succeeded` | `proxy/schema/middleware.rs` L704-L815 | `functions/08-SchemaMiddleware-execution_succeeded.md` |
| R09 | `Context::ready_for_query` | `postgresql/context/mod.rs` L931-L1015 | `functions/09-Context-ready_for_query.md` |
| R10 | `ZeroKms::decrypt_eql` | `proxy/zerokms/zerokms.rs` L388-L469 | `functions/10-ZeroKms-decrypt_eql.md` |
| R11 | `data_row::as_ciphertext` / `to_eql_ciphertext` | `postgresql/rewrite/data_row.rs` L16-L47, L94-L156 | `functions/11-data_row-as_ciphertext-to_eql_ciphertext.md` |
| R12 | `Backend::check_column_config` | `postgresql/middleware/backend.rs` L569-L600 | `functions/12-Backend-check_column_config.md` |

All paths below are relative to `packages/cipherstash-proxy/src/` unless stated otherwise.

## 1. Coverage and limits of this model

- Twelve functions were analysed line by line; everything else is described from the orientation pass or from what those twelve records say about their callees.
- `proxy/schema/eql_domains.rs` and `proxy/encrypt_config` were not read line by line in this pass.
- `proxy/schema/manager.rs` (reload, snapshot build, classification) is described from orientation only.
- The Frontend half is the writer of every queue the Backend consumes; its analysis lives in the first dossier and is only referenced here.
- pg-proto 0.12.0 source is not on disk (the cargo registry holds only `cipherstash-config`), so every statement about framing, authentication, TLS enforcement, operation-id assignment, `Suppress` / `Expand` / `Held` / `ForwardThenFlush` semantics, capacity handling and what an `Err` from the middleware does inside pg-proto is second-hand.
- cipherstash-client 0.42.2 source is not on disk (registry holds `cipherstash-config-0.42.3` and `zerokms-protocol-0.12.31`), so `ScopedCipher::init` / `decrypt`, `KeyHeader::record_with_selector`, `EqlCiphertextV3` deserialisation, `Identifier` equality and `Plaintext::from_slice` are black boxes.
- `ANALYSIS_FORMAT.md` was not found on disk; R08 notes that its prose follows the section order of R06.
- This pass settles two items the first dossier left open about the driver and Backend: the connection is driven by one task with every `forward_*` future awaited to completion before the next is created (R07, orientation), and `ConnectionSettings` has no Frontend writer (R04, R05, orientation).

## 2. Actors and what they reach

| Actor | Trust | Entrypoint | Reaches |
| --- | --- | --- | --- |
| Unauthenticated network peer | untrusted | `postgresql::handler` via `main.rs` L116 (one spawned task per accepted socket); startup, SSLRequest, MD5 authentication inside `intermediary.accept` | Per-connection `ServerConfig` build (re-reads key and cert files for `TlsConfig::Path`), an upstream TCP connect with up to three retries, pg-proto's TLS policy and MD5 credential check, registration in `CANCELLATION_REGISTRY`. No middleware runs before `accept` returns a `Session`. |
| Unauthenticated network peer | untrusted | CancelRequest via `intermediary.accept` (`IntermediaryAccept::CancellationForwarded`) | Lookup in the process-global `CANCELLATION_REGISTRY` and a forwarded cancel to upstream PostgreSQL; the handler then returns. |
| Authenticated client application | semi-trusted | `CipherStashMiddleware::frontend_operation` -> `Frontend::intercept` | Writer of every queue the Backend consumes: `SettingsProtocol` (prepared, portals, journal, pending, boundaries), `ConnectionProtocolState.operations` / portals / prepared statements, the `SchemaMiddleware` execution FIFO and `in_flight_ddl`. |
| Authenticated client application or pooler | semi-trusted | In-band `SET` / `RESET CIPHERSTASH.*`, `DISCARD ALL`, `RESET ALL`, transaction control | Parsed on the frontend path; applied to `ConnectionSettings` (keyset, `unsafe_disable_mapping`, transaction baseline, savepoints) only when the Backend sees a correlated `CommandComplete` (R05, R04). The keyset then selects the `ZeroKms` cipher-cache key and ZeroKMS scope when no default keyset is configured (R10). |
| Authenticated client application | semi-trusted | Result-format codes in Bind; SQL shape determining projection | `Portal::format_codes`, `Statement.projection_columns`, and through them the row re-chunking, `to_sql` encoding and RowDescription OID rewrite (R01, R02). |
| Authenticated client application | semi-trusted | DDL that PostgreSQL confirms | `SchemaMiddleware::execution_succeeded` (overlay, `encrypt_config`, `dirty`), then at Idle readiness `publish_schema_if_changed`, the process-wide publication-pending flag, a `ReloadCommand`, and a catalog reload that every connection later adopts (R08, orientation). |
| Connection pooler multiplexing tenants | semi-trusted | Same as authenticated client | Same state; all settings, savepoints and schema overlays are per Proxy connection, not per pooled tenant. |
| Upstream PostgreSQL | semi-trusted | `CipherStashMiddleware::backend_operation` -> `Backend::intercept` -> `intercept_backend`, once per backend message with pg-proto's optional `OperationId` | Command tags drive deallocation and commit-vs-rollback; terminal messages pop the settings and schema queues; `ErrorResponse` drives journal rollback and `settings.failed`; `ReadyForQuery` status drives transaction settlement and pruning; row bytes reach `as_ciphertext`, `check_column_config`, `ZeroKms::decrypt_eql`, `to_sql` and `rewrite`. |
| Other writers of stored rows and catalog | semi-trusted | Stored column values arriving in `DataRow`; catalog rows read by the reloader | JSON parse of stored bytes (`serde_json::from_slice`), the root-`s` reshape, `EqlCiphertextV3` deserialisation, identifier comparison on the first row, ZeroKMS decrypt requests, and strings from stored bytes inside a client-visible error (R11, R12, R10). Catalog rows reach `build_snapshot` / `classify_column` under operator credentials. |
| Schema reloader tasks | trusted | Periodic `config_reload_interval` task; one spawned task per `ReloadCommand` on the unbounded channel | `coalesced_reload` under a tokio Mutex, `CommittedSchemaStore` ArcSwap publication. This is the only schema state that can change while a handler of a given connection is suspended. |
| ZeroKMS / cipherstash-client | trusted | `ZeroKms::init_cipher` / `encrypt` / `decrypt` / `decrypt_inbound_eql` | Process-wide moka cipher cache keyed by keyset string or `"default"`; network calls awaited with the middleware mutably borrowed. |
| Operator | trusted | Configuration, environment, SIGHUP (`reload_application_config`), TLS material, database credentials | TLS policies both sides, the single username/password used as both downstream MD5 server credentials and upstream client credentials, `connection_timeout`, default keyset, mapping-disabled switch. Existing connections keep their `Context` across SIGHUP. |
| pg-proto 0.12.0 | trusted | Library boundary | Supplies every `OperationId`, interprets every middleware output, owns backpressure (`BoundedPipeline(256)`) and cancellation forwarding (`CancellationPolicy::Forward`). |

Reach that crosses connections:

- A connection's confirmed DDL reaches other connections only through `CommittedSchemaStore` and their own `adopt_latest` / `requires_publication_before_statement` / `needs_publication` calls.
- A connection's keyset selection reaches other connections only through `ZeroKms.cipher_cache` entries, which are shared by key string.
- Stored rows written by one connection (or by a direct database session) reach another connection's Backend as `DataRow` bytes.
- `CANCELLATION_REGISTRY` is process-global and written by pg-proto on accept and by `CancellationRegistrationGuard::drop`.

## 3. State: who reads, who writes, and what that implies

### 3.1 `ConnectionSettings` (`postgresql/context/settings.rs`, inside `ConnectionProtocolState.settings`)

Fields: `keyset` and `unsafe_disable_mapping` as `Scoped` (session value plus optional transaction override), `transaction_start` (lazily captured baseline), `failed`, `savepoints`.

Writers (all on the Backend path, all on the connection task):

- `command_succeeded` via `Context::settings_execution_succeeded` on a correlated `CommandComplete` (R04, R05).
- `execution_failed` via `ConnectionProtocolState::restore_failed_preparation` on every `ErrorResponse`, correlated or not (`context/mod.rs` L264; R03, R09).
- `ready_for_query` via `Context::ready_for_query` on every `ReadyForQuery`, including uncorrelated ones (R09).

Readers:

- Frontend: reads or clones for projection (`may_change_effective`, `project_settings_effects`) with synthetic tags; never writes (R04).
- Backend: `Context::decrypt` reads the keyset immediately before the service call (`context/mod.rs` L1681; R02).
- Metrics: `record_finished_metrics_scope` reads `keyset_identifier()` at log time (R09).

Implied invariants:

- The confirmed settings change only as a function of PostgreSQL's replies, never as a function of what the client sent or what the client was shown.
- `keyset` and `unsafe_disable_mapping` are snapshotted and restored together at every site (R04).
- `failed` reflects PostgreSQL `ErrorResponse` only; a Proxy-local failure (`local_execution_failed`) does not set it (R04, R03).

### 3.2 `SettingsProtocol` (`postgresql/context/settings_protocol.rs`)

Fields: `prepared`, `portals`, `journal` (one rollback `Mutation` per Parse / Bind / Close keyed by `OperationId`), `pending` (queued `SettingsCommand`s per executing operation with `barrier` / `until_ready`), `boundaries`, `settings_since_boundary`, `discarding_until_sync`.

Writers:

- Frontend: `start_prepare`, `prepare`, `capture_literals`, `bind`, `execute`, `simple_query`, `sync`, `boundary`, `close`.
- Backend: `succeeded` (R06), `failed` (R03), `ready` (R09), `prepared_or_bound`, `deallocated`.

Implied invariants:

- A journal entry lives from its Frontend mutation until exactly one of: acknowledgement (`prepared_or_bound`, removed without rollback), failure (`failed`, removed with rollback), readiness (`ready`, removed without rollback), or DISCARD ALL (`succeeded`, removed or expired).
- A `pending` entry lives until: `succeeded` empties a non-`until_ready` entry; `failed` drops it unless `until_ready && barrier` (which survive emptied); or `ready` retires everything at or below the boundary (R03, R06, R09).
- `in_flight()` is true while any pending barrier entry exists, and the driver reads it as its gate (R07).
- Every decision here that uses `<=` or `>` relies on `OperationId` order equalling PostgreSQL processing order (R03, R05, R06, R09).
- `SettingsProtocol.prepared` / `portals` are the name-membership authority for what survives DISCARD ALL in the Context tables (R05, R06).

### 3.3 `ConnectionProtocolState` operation tables (`postgresql/context/mod.rs`)

Fields: `operations`, `prepared_statements`, `portals`, `portal_operations`, `suspended_executions`, `statement_metrics`.

Writers:

- Frontend lifecycle: `begin_parse`, `finish_parse`, `refuse_parse`, `begin_bind`, `finish_bind` / `install_portal`, `execute` / `set_execute_for_portal`, `close`, `register_query`, `refuse_*`, `set_describe`, `set_deallocations`.
- Backend: `preparation_acknowledged`, `close_acknowledged`, `finish_execution`, `discard_operation`, `complete_describe`, `complete_deallocation`, `restore_failed_preparation`, `settings_execution_succeeded` (DISCARD ALL retain), `ready_for_query`.

Implied invariants:

- All of this sits behind one std `RwLock`; no accessor holds a guard across an await (orientation, R05, R09).
- An operation's `ExecuteContext` holds its own `Arc<Portal>`, so removing a portal by name does not change what a running operation decrypts with (R05).
- `operations[op]` is removed by exactly one of: `preparation_acknowledged`, `finish_execution` (Failed, or no describe pending), `discard_operation`, `complete_describe`, or `ready_for_query` (everything at or below the boundary, any status) (R01, R09).
- Portals, portal owners and suspended executions are swept by operation id only at Idle; the unnamed portal of a `completes_at_readiness` execution is the one non-Idle removal, guarded by `Arc::ptr_eq` (R09).
- `prepared_statements` is not touched by readiness; it changes through deallocation (tag-checked), DISCARD ALL (name-membership retain) and journal restore (R05, R09).
- The metrics reference set is journal snapshots, prepared statements, portals, `operations[*].execute` and suspended executions; `operations[*].preparation` is not in it (R05, R09).

### 3.4 Backend-private state (`postgresql/middleware/backend.rs`)

Fields: `encrypted_rows`, `encrypted_rows_operation`, `encrypted_rows_bytes`, `discard_execution`.

Writers: `intercept_backend`, `flush_encrypted_rows`, `decryption_failure`.

Implied invariants (R01, R02):

- A row is held only with `Some(operation)` whose execute context holds `Portal::Encrypted`.
- All held rows share one operation id.
- The buffer is taken before any fallible flush step, so on any exit from a flush the Backend holds no rows.
- `discard_execution == true` implies the buffer is empty at function exit.
- `discard_execution` is cleared at exactly one site, the `ReadyForQuery` arm of the discard branch (L221).

### 3.5 `SchemaMiddleware` (`proxy/schema/middleware.rs`, per connection)

Fields: `base`, `base_encrypt_config`, `encrypt_config`, `resolver` (always an editable `ViaSchemaWithEdits` overlay), `prepared`, `portals`, `executions` (positional FIFO of `Execute(intent)` | `ReadyBoundary`), `in_flight_ddl` + `execution_finished` Notify, `dirty`, `unmodelled`, `local_unmodelled`, `transaction_active`, `savepoints`, `temporary`.

Writers:

- Frontend: `prepare`, `bind`, `execute`, `simple_query`, `protocol_boundary`, `before_statement` / `adopt_latest`, `publication_succeeded` via `prepare_schema_for_statement`.
- Backend: `execution_succeeded` (R08), `execution_failed`, `ready_for_query` (`discard_skipped_executions`, `transaction_ended`, `adopt_latest`), `publication_succeeded` via `publish_schema_if_changed`.
- No other task writes any field.

Implied invariants:

- Every field is a separate lock or atomic written one after another with no single critical section; coherence between them rests entirely on the driver serialising access (orientation, R08).
- `in_flight_ddl` is incremented at push and decremented at removal under the same predicate (`Intent::changes_schema`), at every removal site (R08).
- `execution_succeeded` has no await; its pop, writes and decrement complete without yielding (R08).
- `dirty` is the sole channel from confirmed DDL to `publish_schema_if_changed` (R08).
- `encrypt_config` is read per message by `Context::is_passthrough` and is replaced on the Backend path by `apply_ddl`, savepoint restore, full rollback and `adopt_latest` (R01, R08).

### 3.6 Process-wide state

| State | File | Writers | Readers from a connection |
| --- | --- | --- | --- |
| `CommittedSchemaStore` (ArcSwap snapshot, `requested_publication`, `published_publication`) and `SchemaManager.requested_generation` / `reload_lock` | `proxy/schema/manager.rs` | `coalesced_reload` / `publish_if_newer` on reloader tasks; `mark_publication_pending` from any connection's `publish_schema_if_changed` | `adopt_latest`, `requires_publication_before_statement`, `needs_publication` |
| `ZeroKms.cipher_cache` | `proxy/zerokms/zerokms.rs` | `init_cipher` from any connection's encrypt / decrypt; `Proxy::init` for `"default"` | Same; lookup and insert are not atomic (R10) |
| `CANCELLATION_REGISTRY` | `postgresql/driver.rs` L25-L26 | pg-proto on accept; guard `Drop` (L42) | pg-proto on CancelRequest |
| `ReloadCommand` unbounded mpsc | `proxy/mod.rs` | `Context::reload_schema` from `publish_schema_if_changed` (Backend, Idle readiness) and `prepare_schema_for_statement` (Frontend) | `Proxy::receive`, one spawned task per command |
| `Proxy` (config, schema manager, zerokms) | `main.rs` | `init`; `reload_application_config` on SIGHUP | New connections only |
| Prometheus counters and histograms | several | Backend flush, readiness sweep, metrics scopes | none |

Effects made on these persist after the connection that caused them ends, including when the session ends through a timeout that drops a middleware future mid-await (orientation, R07).

### 3.7 Message-local values that carry meaning across functions

- `operation: Option<OperationId>` from pg-proto is the only correlation between a backend message and Frontend-registered state. `None` is handled differently at each consumer (see 5.3).
- The `CommandComplete` tag bytes are consumed three ways: NUL-stripped exact match for deallocation kinds (`backend.rs` L763-L772), lossy UTF-8 untrimmed for `== "ROLLBACK"` (`backend.rs` L98 -> `settings.rs` L325), and not at all by the schema queue or the settings pop (R01, R04, R06, R08).
- `ReadyForQuery` status is the only transaction-state signal used at readiness, and is not cross-checked against the commit/rollback decision made earlier from the tag (R04, R09).
- The first held row's column count becomes `result_column_count` for the whole flushed batch (R02).
- The first byte of a stored column value selects binary versus text JSON parsing, independent of the portal's format code (R11).

## 4. End-to-end flows

### 4.1 Connection setup (R07)

1. `main.rs` accept loop spawns one tracked task per socket running `postgresql::handler`; its result is only logged, and a panic in the task is not otherwise handled there.
2. `validate_downstream_tls` rejects `require_tls` without TLS config.
3. Downstream policy: `Required` iff TLS config present and `require_tls`; `Optional` iff present and not required; `Disabled` iff absent.
4. Upstream policy: `Disabled` iff `development.disable_database_tls`; else `VerifyFull` iff `database.with_tls_verification`; else libpq `Prefer` (the default, since `with_tls_verification` defaults to false).
5. The database username and password are used both as downstream MD5 server credentials and as upstream client credentials; there is no separate Proxy credential.
6. `Route::resolve` ignores `StartupParameters`; the upstream target is always the configured host and port.
7. One `Intermediary` is built: `BoundedPipeline(256)`, `CancellationPolicy::Forward`, the global registry, `CipherStashMiddlewareFactory`.
8. `intermediary.accept` runs under the optional `connection_timeout`; it either returns a `Session` or `CancellationForwarded`.
9. A `CancellationRegistrationGuard` is created only after a `Session` exists (L125); from then on the key is detached on every exit.
10. `CipherStashMiddlewareFactory::create` builds one `CipherStashMiddleware { frontend, backend }` from a cloned `Context`; all mutable Context state is behind Arcs, so both halves and the handler's own Context see the same state.

### 4.2 Forward loop, capacity retry and outcome drain (R07)

1. Primary loop: `session.forward_next()` under the timeout; one message in either direction per iteration; direction selection is pg-proto's.
2. On `FrontendProjectionError::Capacity`: alternate `forward_backend` then `forward_frontend` until the retained frontend message is accepted. At least one backend message is forwarded before each frontend retry. This loop has no Copy-in exit.
3. After a forwarded `FrontendExpanded`, `Query` or `Sync`, if `schema_ddl_in_flight() || settings_in_flight()` at that moment: the outcome drain calls `forward_backend` only, until both flags clear, or a `CopyInResponse` / `CopyBothResponse` is seen, or an error, or a timeout.
4. The drain runs after the triggering message was intercepted, so what it protects is the next frontend message.
5. On the Copy early exit the flags are still set, and `CopyData` / `CopyDone` / `CopyFail` do not re-enter the drain.
6. A DDL-only drain ends at `CommandComplete` (where `in_flight_ddl` is decremented), before the `ReadyForQuery` that runs `became_ready`.
7. Every `forward_*` await uses `?`; a timeout or error ends the session and no further middleware call is made on it.
8. Middleware errors surface as the typed `Error`; every other pg-proto error is flattened to `io::ErrorKind::InvalidData`.
9. `connection_timeout` wraps accept and each `forward_*` await; it is `None` only for an explicit `0` (default 120000 ms, R07), in which case those awaits, the drain and the capacity loop are unbounded.

### 4.3 Backend message pipeline (R01)

Order of steps for every backend message:

1. Deallocation: if the message is a `CommandComplete` whose NUL-stripped tag matches one of three deallocation tags, `Context::complete_deallocation` applies the next queued deallocation when its kind matches; unknown operation or empty queue is `Ok` with no change.
2. Acknowledgements: ParseComplete / BindComplete -> `preparation_acknowledged` (journal entry removed, then `operations[op]`; `UnknownOperation` after the journal removal if absent); CloseComplete -> `close_acknowledged`.
3. Mode selection, in this order: passthrough (`is_passthrough()`, evaluated per message from the live `encrypt_config` and operator switch), then discard (`discard_execution`), then normal.
4. Passthrough: `report_outcome`, `became_ready` on ReadyForQuery, forward unchanged. No row is held or decrypted, no description is rewritten, and neither `discard_execution` nor `encrypted_rows` is consulted.
5. Discard: `report_outcome(discarded = true)`; ReadyForQuery clears the flag, runs `became_ready` and is forwarded; everything else is `Suppress`.
6. Normal, non-DataRow with rows held: flush first; on flush failure set `discard_execution`, call `local_execution_failed`, call `report_outcome(discarded = true)` only when `operation.is_some()`, and return the error response without running the rest (including `became_ready` when the trigger was a ReadyForQuery).
7. Normal: `report_outcome` once, then per-message handling: DataRow (hold or forward), ParameterDescription and RowDescription rewrites, terminal arms, ReadyForQuery -> `became_ready`, default forward.
8. `report_outcome` -> `execution_completed` runs settings (Command only, correlated only), then schema (unconditional pop), then `finish_execution` or `discard_operation`; `postgres_failed` runs journal rollback plus `settings.failed`, then schema failure pop, then `finish_execution(Failed)` which is the only source of a replacement diagnostic.
9. Settings and schema mutations land before the fallible `finish_execution`; an `Err` return does not undo them.

### 4.4 Result rows: hold, flush, decrypt, rewrite (R01, R02, R11, R12, R10)

1. `data_row_handler` -> `get_portal_from_execute`: `Ok(false)` for operation `None`, `Err` for unknown operation or missing execute, `Ok(true)` only for `Portal::Encrypted`.
2. A held row's operation is compared with `encrypted_rows_operation`; a mismatch goes to `decryption_failure` without `local_execution_failed`.
3. Limits (4096 rows, 64 MiB of column bytes) are tested after the push; reaching one triggers a flush.
4. Flush takes the buffer, re-resolves the portal from the operation's execute context (a second, independent lookup; only the variant is re-checked), and reads `result_column_count` from the first row.
5. `as_ciphertext` per row: zip of row columns with `projection_columns`, yielding `min(row width, P)` entries; `Some` only under a `Some` config, non-NULL column and successful parse; parse failure is logged and becomes `None`.
6. `to_eql_ciphertext`: leading `0x01` stripped as binary jsonb; otherwise text JSON; empty value errors; a root-level `"s"` key causes `s` / `c` / `a` / `op` to be moved under a new one-entry `sv` array with root `k` and `sv` overwritten; then `serde_json::from_value::<EqlCiphertextV3>`.
7. `check_column_config`: zip of `projection_columns` with the flat all-rows ciphertext vector, so `min(P, total)` pairs are examined; identifier equality for `(Some, Some)`; `(Some, None)` and `(None, None)` accepted; error text for `(None, Some)` comes from the stored payload's identifier.
8. `Context::decrypt` re-reads the confirmed keyset and delegates with no length check; `ZeroKms::decrypt_eql` in `RootOnly` mode requires some keyset, acquires the cipher from the cache, builds one record per scalar or per document root (`sv[0]`), calls `cipher.decrypt` with `DecryptOptions::default()`, and maps outputs back by position with a zip.
9. `add_decrypt_duration_for_execute` can return `Err` after a successful decrypt, which drops the batch.
10. `plaintexts.chunks(result_column_count)` zipped with rows; `to_sql` encodes by `Plaintext` variant and format code; `rewrite` overwrites only non-NULL columns with a `Some` plaintext and rejects only more plaintexts than columns.
11. Any failure on any row or value discards the whole batch of that call.

### 4.5 Settings change: from client statement to confirmed state (R04, R05, R06, R07)

1. Frontend classifies statements (`SettingChange::parse`, `SettingsCommand::from_statement`) and queues them in `SettingsProtocol.pending[op]` with `barrier` computed by `may_change_effective` against then-confirmed settings.
2. The driver drain holds further frontend messages while any barrier entry is pending.
3. On a correlated `CommandComplete` the Backend calls `settings_execution_succeeded` before consulting `discarded`: operation `None` returns `Ok` with no change; otherwise `succeeded` pops one command positionally and ORs `barrier` into `settings_since_boundary`.
4. `command_succeeded` captures the transaction baseline (if absent) before mutating, then applies the command: Commit | Rollback uses the tag only for `== "ROLLBACK"` and then ends the transaction; Savepoint pushes unconditionally; RollbackTo / Release look up by exact string and are silent no-ops on a miss; Change assigns by scope; ResetAll preserves baseline, `failed` and savepoints; DiscardAll replaces the whole struct with `Default`.
5. If the popped command is DiscardAll, `succeeded` prunes `prepared` / `portals` / `journal` by operation id and the caller prunes the Context tables by name membership, then sweeps all metrics scopes.
6. The write lock is released before metrics are recorded.

### 4.6 PostgreSQL error (R01, R03, R09)

1. `report_outcome` on `ErrorResponse` -> `postgres_failed` on the passthrough, discard, flush-failure (correlated only) and normal paths.
2. `restore_failed_preparation` -> `SettingsProtocol::failed`: for `None`, nothing is written there; otherwise the skipped range is `[operation, nearest recorded boundary >= operation]`, or open-ended with `discarding_until_sync` set.
3. Skipped pending entries are dropped (except `until_ready && barrier`, emptied); skipped journal entries are undone in descending id order, with rebase onto a later same-name same-kind entry when one exists, and with the previous unnamed object discarded only for an unnamed non-Close entry at the failed operation itself.
4. The caller applies the returned restores in order (newest first) and sets `settings.failed` unconditionally.
5. The schema queue pops one entry as failed.
6. `finish_execution(op, Failed)` returns the stored Proxy diagnostic for that operation if any; `UnknownOperation` is returned after the settings and schema changes if there is no operations entry.

### 4.7 Local decryption failure and discard mode (R01, R02)

1. `decryption_failure` clears the buffer fields, sets `discard_execution`, and returns `Expand([ErrorResponse])`.
2. Until a `ReadyForQuery` passes through the discard branch, every other message is suppressed, while PostgreSQL's outcomes are still committed to settings, the schema queue, deallocations and acknowledgements.
3. If the message that triggered the failed flush was itself a `ReadyForQuery`, it is not forwarded and `became_ready` is not run for it (R01, R09).
4. Passthrough is tested before discard, so a message arriving while `is_passthrough()` is true does not consult or clear `discard_execution`.

### 4.8 ReadyForQuery (R09, R08, orientation)

`Context::became_ready` runs three steps in order:

1. `Context::ready_for_query(status, boundary)`: `ConnectionSettings::ready_for_query` on every call; `SettingsProtocol::ready` clears `settings_since_boundary` and `discarding_until_sync` on every call; with `boundary == None` nothing else changes; with `Some(b)` everything at or below `b` leaves `operations`, `pending`, `journal`, `boundaries` for any status, and portals / owners / suspended executions leave only at Idle. Pending commands retired here are dropped unapplied and journal entries are not rolled back.
2. If Idle: `publish_schema_if_changed` (awaited; marks publication pending, sends `ReloadCommand`, awaits a oneshot, adopts on return). A failure returns `Err` before step 3 and before the `ReadyForQuery` is forwarded.
3. `SchemaMiddleware::ready_for_query`: `discard_skipped_executions` to the next `ReadyBoundary`, `transaction_ended`, `adopt_latest`.

### 4.9 DISCARD ALL: two independent passes on one message (R01, R05, R06)

1. Tag-driven: `is_deallocation` / `removes_all_statements` select `complete_deallocation`, which cross-checks the queued deallocation kind against the tag and removes only prepared entries whose recorded Parse precedes the operation (an entry with no Parse is kept).
2. Command-driven: `succeeded` pops the queue head; if it is `Change(DiscardAll)` the settings reset and the name-membership prunes run, regardless of the tag.
3. The Frontend decides each queue separately from two matches on `Statement::Discard { ALL }` (`settings.rs` L34-L41 and `frontend.rs` L1458-L1475); nothing shared enforces that they agree.
4. SchemaMiddleware's own `prepared` / `portals` maps are a third store whose DISCARD ALL handling was not established (see section 10).

### 4.10 DDL outcome and schema publication (R08, R07, orientation)

1. Frontend pushes `Execute(intent)` entries (one per Execute, one per parsed statement of a simple Query) and `ReadyBoundary` entries (simple Query, Sync, refused Query), incrementing `in_flight_ddl` for `changes_schema()` intents.
2. Each terminal message (`CommandComplete` with any tag, `EmptyQueryResponse`, `PortalSuspended`) pops the front `Execute` entry by position, with no operation id, tag or end kind supplied.
3. Arms by the client statement: savepoint push; release and rollback-to with truncation; full rollback (fresh overlay, base `encrypt_config`, `dirty = false`, `unmodelled = false`, `temporary.failed = true`); DISCARD; modelled DDL (`split_change` into temporary and catalog parts, `apply_ddl` only when `models_ddl` is true at completion time); otherwise `mark_unmodelled`.
4. Tail: `dirty` update, `in_flight_ddl` decrement, `notify_waiters`.
5. At Idle readiness `dirty` leads to process-wide publication, a catalog reload under operator credentials in one REPEATABLE READ READ ONLY transaction, and `adopt_latest` on this connection; other connections adopt at their own next adoption point.
6. `wait_for_ddl` on the Frontend path awaits `in_flight_ddl == 0` with no internal timeout while the middleware is exclusively borrowed.

### 4.11 Keyset to cipher (R10, R05)

1. The keyset argument on the read path is the connection's confirmed setting at call time.
2. `decrypt_eql` requires either that argument or a configured default before acquiring a cipher, even for an all-`None` input.
3. `init_cipher` keys the cache by the identifier's Display string or the literal `"default"`; a hit makes no ZeroKMS call; a miss calls `ScopedCipher::init` with the connection keyset if given, else the configured default.
4. `Context::validate_setting` on the frontend path is what keeps the keyset argument `None` whenever a default keyset is configured; `decrypt_eql` itself gives the argument precedence.
5. The ciphertext-keyset-equals-cipher comparison is applied in `AllEntries` mode (inbound validation) and not in `RootOnly` mode (database reads).

## 5. Cross-function invariants

These hold, or are relied on, only across functions; no single record could state them.

### 5.1 One task, one borrow: the serialisation every other invariant rests on

- The driver runs one sequential loop per connection and awaits each `forward_*` future to completion before creating the next (R07).
- Both `IntermediaryMiddleware` methods take `&mut self` on the single `CipherStashMiddleware`, so a Backend intercept cannot run while a Frontend intercept future is alive, and vice versa (orientation, R05).
- Consequently: no protocol-state lock ordering exists to reason about; multi-lock sequences in `SchemaMiddleware` and the split lock acquisitions in `ready_for_query` (release at L1004, re-acquire at L1179) are coherent only because of this (R08, R09).
- Consequently: while a handler awaits ZeroKMS or schema publication, nothing else on the connection progresses; only `CommittedSchemaStore`, the cipher cache and other process-wide state can change underneath it.
- The only way a middleware future is dropped mid-await is the timeout wrapper, which ends the session; per-connection state is then discarded and process-wide effects already made persist (orientation, R07).
- What this does not give: ordering between frontend and backend messages across handler invocations. A frontend message can be intercepted between two backend messages of one execution wherever the driver is in `forward_next` rather than in the drain.

### 5.2 Three positional queues are advanced by one backend message stream under three different pairing rules

| Queue | Key | Pop trigger | Tag checked | `operation == None` |
| --- | --- | --- | --- | --- |
| `SettingsProtocol.pending[op].commands` (R06, R05) | operation id, then position | `CommandComplete` only | Only `== "ROLLBACK"` after the pop, for Commit / Rollback | No pop, no change |
| `SchemaMiddleware.executions` (R08) | none; connection-wide position, stops at `ReadyBoundary` | `CommandComplete`, `EmptyQueryResponse`, `PortalSuspended`, and (as failed) `ErrorResponse` | Never | Pops anyway, except on the flush-failure path (`backend.rs` L244) |
| Deallocation queue (R01) | operation id | `CommandComplete` with a deallocation tag | Kind cross-checked against the tag | `Ok`, no change |

- The three stay in step only if the sqltk statement count for an operation equals PostgreSQL's terminal-message count, the terminal kinds line up with each queue's trigger set, and correlation is correct.
- `EmptyQueryResponse` and `PortalSuspended` advance the schema queue and not the settings queue.
- A surplus settings command is dropped unapplied at readiness (R09); a surplus schema entry is discarded unapplied at the next boundary (R08); a shortfall in either returns `None` silently.
- Because settings then schema then `finish_execution` run in that order with `?` between them, a settings `Err` (poisoned lock only) skips the schema pop for that completion (R08).

### 5.3 `operation == None` has a different meaning at every consumer

- `data_row_handler`: row forwarded unchanged (R01).
- `settings_execution_succeeded`: command not applied (R05).
- `SettingsProtocol::failed`: no journal rollback and no skip started, while `settings.failed` is still set (R03).
- `SchemaMiddleware::execution_succeeded` / `execution_failed`: front entry popped as usual (R08); skipped only on the flush-failure path (R01).
- `ready_for_query`: settings transaction settlement and the two flags are still processed; nothing is pruned (R09).
- `complete_deallocation`: no change (R01).
- Driver gate: a `None` on the message that would clear a flag leaves the flag set, and the drain continues until another exit condition (R07).

### 5.4 Three boundary registries are driven by one ReadyForQuery

- `SettingsProtocol.boundaries`: pushed for Sync, simple Query and FunctionCall on the Frontend path (R03); not pushed for messages suppressed in Frontend discard or for COPY sub-protocol messages.
- `SchemaMiddleware` `ReadyBoundary`: pushed by `simple_query`, `Context::sync` and `refuse_query`; R08 records that no schema boundary is pushed for FunctionCall with mapping enabled (`frontend.rs` L333-L335, `lifecycle.rs` L75-L89).
- Context operation pruning: by the `boundary` id pg-proto attaches to the `ReadyForQuery`, with no membership check against `SettingsProtocol.boundaries` (R09).
- So the same `ReadyForQuery` can retire a settings boundary by id, drain the schema FIFO to a positional marker pushed (or not pushed) by a different rule, and prune operations by id comparison.
- `SettingsProtocol::failed` treats any recorded boundary at or after the failed operation as the end of PostgreSQL's skip, in one undifferentiated list (R03).

### 5.5 Fixed commit order on a backend message, with no undo

- Order: deallocation, acknowledgement, mode selection, flush of held rows, settings commit, schema pop, `finish_execution` / `discard_operation`, per-message handling, `became_ready` (R01, 4.3).
- Rows held for an operation are flushed before the triggering message's own outcome is committed, so a flush triggered by a message decrypts under the settings confirmed up to but not including that message.
- Each step's state change survives a later step's `Err`; the model depends on the driver ending the session on `Err` (R07) so that partial state is not reused.
- `became_ready` is itself three ordered steps (4.8), and a publication failure leaves the protocol sweep done and the schema readiness step not done.

### 5.6 Confirmed state follows PostgreSQL, not what the client was shown

- In discard mode and on the flush-failure path the client sees an error and suppressed messages, while settings commands, schema DDL, deallocations and acknowledgements that PostgreSQL confirmed are applied (R01, R05, R08).
- `settings.failed` is set only by PostgreSQL `ErrorResponse`; a Proxy-local failure leaves it clear, so later `CommandComplete` messages on the discard path still apply `Change` commands (R04).
- In passthrough mode every outcome step still runs; only row handling and description rewriting are skipped (R01).

### 5.7 Transaction end is decided twice from two different signals

- At the command: Commit | Rollback takes the rollback path iff the untrimmed lossy tag equals `"ROLLBACK"`, then clears baseline, overrides, `failed` and savepoints, so a later Idle readiness has nothing left to restore (R04).
- At readiness: Idle restores the baseline iff `failed` is set, then ends the transaction; non-Idle writes nothing, so `failed` persists (R09).
- `failed` is connection-wide, set by any `ErrorResponse` including uncorrelated ones, and cleared only by a found RollbackTo, `end_transaction` or `reset` (R04, R09).
- The baseline is captured lazily on the first confirmed command of any kind and is discarded by DiscardAll in the same call that captured it (R04).
- Nothing compares the tag decision with the following `ReadyForQuery` status.
- The schema side has a third, independent view: `SchemaMiddleware` observes full ROLLBACK by client statement, has no arm for COMMIT, and settles `temporary` at Idle readiness (R08).

### 5.8 Two savepoint stacks with separate miss handling

- `ConnectionSettings.savepoints`: pushed unconditionally, last-match lookup by exact string, miss is a silent no-op (R04).
- `SchemaMiddleware.savepoints`: same naming function (`postgres_identifier`), miss handled by `mark_unmodelled(false)`; also cleared by `adopt_latest`, whose guards read `transaction_active` (R08).
- Both are fed from the same positional completions but by different queues (5.2), and neither is compared with the other.

### 5.9 Row alignment chain: every link zips to the shorter side

`projection_columns` (fixed at Parse / Query from the mapper) -> each row's columns (`as_ciphertext` zip) -> flat ciphertext vector across rows -> `check_column_config` zip against one copy of `projection_columns` -> decrypt (length preserved by `ZeroKms`) -> `chunks(first row width)` zip rows -> `format_codes` zip per chunk -> `rewrite`.

- The only length comparison in the chain is `rewrite` rejecting more plaintexts than columns (R02, R11).
- Chunk `n` corresponds to row `n` only when every row has the first row's width and `projection_columns.len() >= ` that width (R02).
- `check_column_config` examines the first row only, and only when that row is at least `P` wide are the pairs aligned (R12).
- `RowDescription` rewriting zips `projection_columns` with fields independently, with no length comparison, and advertises `col.postgres_type`, while `to_sql` encodes by the decrypted `Plaintext` variant (R01, R02).
- The portal consulted is the operation's single portal; on the Bind path that is one statement per portal, and for a multi-statement simple Query it is one portal for every statement's rows (R02, R12; first dossier 5.16).

### 5.10 Read-path acceptance is a conjunction spread over four functions, and differs from the inbound path

For a stored value to be replaced by plaintext on the way to the client:

- its projection entry is `Some` (R11);
- it is non-NULL and parses as `EqlCiphertextV3`, possibly after the root-`s` reshape (R11);
- if it is in the first row, its identifier equals the configured column's (R12);
- some keyset is available and `cipher.decrypt` succeeds for the whole batch (R10);
- the decrypted bytes decode through `Plaintext::from_slice` and encode through `to_sql` (R02, R10).

Outcomes other than replacement or batch error:

- A configured non-NULL column that fails to parse yields `None`, is accepted by `check_column_config`, skipped by `decrypt_eql`, and left with its stored bytes by `rewrite` (R01, R02, R11, R12).
- A column whose projection entry is `None` is never parsed.

The inbound (`AllEntries`) path applies the ciphertext-keyset comparison, all-entry authentication, `validate_storage_metadata` and `sem_terms_match`; the read (`RootOnly`) path applies none of these in the Proxy and relies on `cipher.decrypt` (R10; first dossier 5.9).

### 5.11 Keyset coherence across phases

- The keyset used to decrypt is read at flush time; neither `Portal` nor `ExecuteContext` stores one (R02).
- The only commit point of a client-selected keyset is a correlated `CommandComplete` (R05).
- The driver drain keeps later frontend messages from being intercepted while a barrier settings command is pending, which is what lets the Frontend read confirmed rather than speculative settings (R07; first dossier 5.8).
- That protection has the drain's exit conditions: it ends early on Copy-in / Copy-both with flags still set, and is entered only after `FrontendExpanded`, `Query` or `Sync` (R07).
- Flush-before-outcome (5.5) means rows of an operation are decrypted under settings confirmed before that operation's terminal message, including commands of earlier statements of the same simple Query.
- The cipher for that keyset comes from a process-wide cache keyed by string, shared with every other connection (R10).

### 5.12 Portal classification and passthrough are two independent decisions

- Whether a row is held depends on the portal variant chosen by the Frontend at Bind / Query time (R01).
- Whether the Backend looks at portals at all depends on `is_passthrough()` evaluated per message from the live `encrypt_config` and the operator switch; it ignores the `unsafe_disable_mapping` setting (R01).
- `encrypt_config` changes on the Backend path (`apply_ddl`, savepoint restore, full rollback, `adopt_latest` at Idle, return from publication) while later pipelined operations may already hold portals (R01, R08).
- Passthrough is tested before `discard_execution` and before the held-row check, so a transition to passthrough bypasses both (R01).

### 5.13 The driver gate, the in-flight flags and `wait_for_ddl` form one mechanism

- Flags are raised by Frontend predicates (`SettingsCommand::may_change_effective` -> `barrier`; `Intent::changes_schema` -> `in_flight_ddl`) and lowered only by Backend outcomes on the same task (R07, R08).
- The drain is the only place the driver reads backend messages without also accepting frontend messages.
- `wait_for_ddl` suspends a Frontend handler with the middleware borrowed; nothing on the connection can lower `in_flight_ddl` during that await, so it terminates only if the counter was already zero or the session times out (orientation, R07).
- Therefore `wait_for_ddl` terminating depends on every path into a Frontend handler having passed through a drain that exited with flags clear; R07 records the Copy early exit and the capacity loop as paths not covered by that.
- Completeness of the two predicates decides which outcomes later frontend work can observe before they are confirmed (R07).

### 5.14 Name-membership pruning versus operation-id pruning

- DISCARD ALL: `SettingsProtocol` prunes by operation id (`> operation` survives, plus the unnamed prepared entry); Context tables are then pruned by whether the name still exists in `SettingsProtocol`, ignoring the id stored in the pruned map (R05, R06).
- Deallocation: Context `prepared_statements` pruned by recorded Parse id, keeping entries with no Parse (R01, orientation).
- Readiness: by operation id for operations / pending / journal / boundaries, and at Idle for portals (R09).
- Journal restore: remove-then-reinstall by name with no check that the live entry's operation equals the undone id (R03).
- These four rules agree only if `SettingsProtocol.prepared` / `portals` and the Context tables hold the same name sets; R05, R06 and R09 each record cases where they are written by different paths (simple-Query unnamed portal; Bind when the statement is absent from `SettingsProtocol.prepared`; entries with no recorded Parse; `restore_failed_preparation` writing portals and owners as independent Options).

### 5.15 Discard mode needs a ReadyForQuery that passes through the discard branch

- `discard_execution` is cleared only at `backend.rs` L221.
- When the failed flush was triggered by a non-terminal message or by the row cap, the eventual `ReadyForQuery` clears it and is forwarded.
- When the trigger was the `ReadyForQuery` itself, that message is consumed by the failure return and `became_ready` is not run for it (R01, R09).
- R01 also records that `local_execution_failed` at L238-L239 can return `Err` after `discard_execution` was set at L237.

### 5.16 Schema change propagation crosses connection scope at one point

- Confirmed DDL changes the connection's overlay and `encrypt_config` immediately at the terminal message (R08).
- It becomes visible to other connections only after this connection's Idle readiness publishes and a reloader task swaps the snapshot (orientation).
- A DDL-only drain ends at `CommandComplete`, before that readiness (R07), so the next frontend message of the same connection can be intercepted against the local overlay while publication has not started.
- `dirty` set to false by a confirmed full ROLLBACK relies on the overlay holding nothing that was committed earlier in the same readiness window; no arm observes COMMIT (R08).
- A change confined to the temporary layer does not set `dirty` (R08).

### 5.17 Error text and metrics leave the trust boundary from Backend-held data

- `error_to_response` maps `Held*` protocol errors to a generic system error and everything else to `err.to_string()` (R01).
- `ColumnConfigurationMismatch` takes the catch-all arm; in the `(None, Some)` arm its table and column strings come from stored bytes (R12).
- `log_deserialise_error` logs the serde_json error string at debug with the projection column's names (R11).
- The slow-statement warning reads the keyset at log time, after readiness settlement (R09).

## 6. Places where records disagree or differ in confidence

1. **What a middleware `Err` does.**
   R01 records "A middleware Err terminates the connection" as established by nothing found in this repository (pg-proto behaviour).
   R07 records that the driver propagates middleware errors with `?` (L164-L167, L204-L207, L211-L212, L253-L256) and never uses the session again.
   The orientation note for `backend.rs` says any `Err` ends the connection via the driver.
   Both stand: the driver half is established in-repo by R07; what pg-proto has already written to either socket before returning the `Err` is not.

2. **Whether Frontend handlers can run during the decrypt await.**
   R02 lists "The operation still has an ExecuteContext after the decrypt await" as established by nothing, and carries an open question about pg-proto's task model during the await at `backend.rs` L522.
   R05, R07 and the orientation establish `&mut self` exclusivity and a single sequential task, under which nothing on the connection runs during that await.
   R01's related open question is different in kind: whether a frontend message can be intercepted between two backend messages of one execution, which exclusivity does not exclude and R07 leaves to `forward_next` direction selection.

3. **FunctionCall boundaries.**
   R03 records that every Sync and simple Query / FunctionCall the Frontend forwards pushes a settings boundary (`lifecycle.rs` L62, L75-L77, L394; `settings_protocol.rs` L284).
   R08 records that for FunctionCall with mapping enabled no schema `ReadyBoundary` is pushed (`frontend.rs` L333-L335, `lifecycle.rs` L75-L89).
   R03 also notes a test comment at `settings_protocol.rs` L811-L812 saying FunctionCall / config-disabled Query record no boundary, against `lifecycle.rs` L58-L63, L75-L89 which record one.
   These are about two different registries (5.4); the intended contract for each is unresolved.

4. **Who removes barrier entries.**
   R07 states as an assumption that an `until_ready` barrier entry is cleared only by `ready()` with id at or above its own (`settings_protocol.rs` L459), and elsewhere lists removal sites L292, L354-L364 and L459.
   R03 records that `failed` drops skipped pending entries except `until_ready && barrier`, which survive with commands emptied.
   R06 records that `succeeded` removes a non-`until_ready` entry when it empties.
   Read together: `until_ready` barrier entries leave only at readiness; other barrier entries can also leave through `succeeded` or `failed`; R07's open question about an Execute barrier entry that ends in `EmptyQueryResponse` or `PortalSuspended` (which skip `succeeded`) remains.

5. **The connection timeout default.**
   The orientation says "when None those awaits are unbounded".
   R07 records that `None` arises only for an explicit `0` and the default is 120000 ms (`config/database.rs` L82).
   Not contradictory; R07 is the more specific.

6. **What a `None` ciphertext under a configured column means.**
   The comment at `backend.rs` L585 (cited in R12) treats it as SQL NULL.
   R11 records that `as_ciphertext` also yields `None` when `to_eql_ciphertext` fails on present bytes, and that the `ColumnIsNull` arm has no producer.
   R02 lists "A configured column whose bytes do not deserialise as EQL is an error" as established by nothing.

7. **Scope of `check_column_config`.**
   R01 and R02 describe it as covering "the first row of the batch".
   R12 refines this: exactly the first row's columns `0..P` only when that row is at least `P` wide; when narrower, pairs are misaligned across the row boundary and the `(None, Some)` arm becomes reachable.
   R10 cites the first-row-only coverage second-hand from R02.

8. **Tag handling.**
   R01 records deallocation tags matched after stripping one trailing NUL.
   R04, R05 and R06 record that the same bytes reach `== "ROLLBACK"` untrimmed and lossily decoded.
   The records agree on the code; whether the payload carries a NUL is unresolved in all four.

9. **Whether the terminal-arm diagnostic substitution can fire.**
   R01 records that a stored Proxy diagnostic replaces a message only via `finish_execution(Failed)`, which `report_outcome` requests only for `ErrorResponse`, so the substitution in the terminal arm cannot fire with the current callee.
   R01's own open question asks whether PostgreSQL always answers a refused Execute / Query with an `ErrorResponse` on the same operation.
   Same record, two confidence levels: the code path is established, the upstream behaviour it depends on is not.

10. **Unmapped mode and the journal.**
    R03 records that `unmapped_parse` / `unmapped_bind` write no journal entries and `close` is guarded by `!mapping_disabled`.
    R05 records that with operator-level `mapping_disabled` no Pending carries a command.
    R03 leaves open whether session-level `unsafe_disable_mapping` still writes journal entries; R09 leaves open whether mapping-disabled paths leave `SettingsProtocol.portals` and Context portals with different key sets.

11. **Flush-failure outcome reporting.**
    R05 lists `backend.rs` L245 as a call site of `report_outcome` (flush failure, `discarded = true`) without qualification.
    R01 and R08 record that it runs only when `operation.is_some()`.
    R01 and R08 are the more specific.

## 7. Where complexity concentrates

1. **`Backend::intercept_backend` mode and ordering logic** (`backend.rs` L156-L377).
   Three modes tested in a fixed order, a flush interposed before outcome reporting, four call sites of `report_outcome` with different `discarded` and `None` handling, and one path that consumes a `ReadyForQuery`.
2. **The positional pairing of backend completions with Frontend-built queues** (`settings_protocol.rs` L287-L325, `schema/middleware.rs` L704-L815, L939-L960, `context/mod.rs` L861-L905).
   Three queues, three keys, three trigger sets, one of them tag-checked (5.2).
3. **`SettingsProtocol::failed` and `restore_failed_preparation`** (`settings_protocol.rs` L331-L432, `context/mod.rs` L216-L266).
   Skipped-range selection from an undifferentiated boundary list, descending undo with rebase, an unnamed-object rule, and a caller that applies restores by remove-then-reinstall across five Context tables.
4. **DISCARD ALL** (`settings.rs` L356, `settings_protocol.rs` L294-L322, `context/mod.rs` L1583-L1598, `backend.rs` L163-L170).
   Two passes selected by different signals, id-based pruning in one store and name-based pruning in another, whole-struct reset of settings including baseline and savepoints.
5. **Transaction settlement** (`settings.rs` L317-L386, L407-L418; `context/mod.rs` L941; `schema/middleware.rs` L765-L777, L449-L459).
   Lazy baseline, tag string decision, connection-wide `failed`, readiness status, and a separate schema-side model, with no cross-check among them.
6. **Row alignment and batch flush** (`backend.rs` L473-L600, `rewrite/data_row.rs`, `context/portal.rs` L56-L73, `data/to_sql.rs`).
   A chain of unchecked zips over lengths that originate in four places (mapper projection, PostgreSQL row width, client format codes, cipher output), with `chunks(first row width)`, `unreachable!()` on `Passthrough` and `unimplemented!()` for binary `BigUInt`.
7. **Stored-byte classification** (`rewrite/data_row.rs` L94-L151).
   Format chosen by first byte, shape chosen by presence of a root `"s"` key, validation delegated to a black-box deserialiser, failure folded into `None`.
8. **The driver's secondary loops** (`driver.rs` L140-L270).
   Capacity retry without a Copy exit, an outcome drain whose entry and exit conditions depend on Frontend predicates and pg-proto's message classification, and a single optional timeout as the only bound.
9. **`SchemaMiddleware` field-by-field state** (`proxy/schema/middleware.rs`).
   Around a dozen independently locked fields, classification recomputed at completion time against state that may have changed since prepare time, a temporary-table layer with its own `failed` / `committed` flags not covered by checkpoints, and `adopt_latest` clearing savepoints under guards that read a flag another batch's readiness overwrites.
10. **`ready_for_query` sweep** (`context/mod.rs` L931-L1015).
    Unconditional removal at or below a boundary supplied by pg-proto, different rules for Idle and non-Idle, metrics candidates computed from what was removed, lock released and re-taken, and behaviour for `boundary == None` that still settles settings.
11. **Keyset scope and the cipher cache** (`zerokms.rs` L147-L232, L388-L469).
    Cache keyed by rendered string with a reserved literal, non-atomic lookup and insert, mode-dependent keyset comparison, and position-defined SteVec root.
12. **Connection setup policy** (`driver.rs` L85-L128, L284-L363).
    Shared credentials both sides, upstream TLS default `Prefer`, per-connection TLS material reload, and a cancel-key guard that exists only after `accept` returns.

## 8. Unenforced assumptions (verbatim from the records)

Each line below was recorded with "established by: nothing found" (in some cases with a note on where an enforcing line would sit).
They are carried verbatim and are assumptions, not findings.

### Backend::intercept_backend

- pg-proto gives every DataRow of an encrypted execution the Some(operation) of its Execute/Query; with None the row is forwarded unchanged.
- is_passthrough() at Backend time agrees with the Encrypted/Passthrough classification the Frontend gave the portal; it is re-evaluated per message from the live encrypt_config and ignores the portal.
- A row sequence is terminated by CommandComplete, PortalSuspended or ErrorResponse before any ReadyForQuery and before any row of another or uncorrelated operation.
- Uncorrelated terminal and error messages belong to the front entry of the connection-wide schema execution FIFO.
- ReadyForQuery carries Some(operation) of the Sync/Query it answers; with None no operations, portals or suspended executions are pruned.
- Every ErrorResponse, correlated or not, means the current transaction's settings must be restored at the next Idle readiness.
- A middleware Err terminates the connection, so partial state left by any `?` is never reused.
- On an operation mismatch between held rows and a new encrypted row, neither operation's execute context needs finishing locally.
- projection_columns.len() equals each held row's column count and that count is non-zero (chunks(0) panics).
- Column-identifier agreement checked on the first row holds for every row in the flushed batch.
- Suppress(msg) withholds the message from the client and Expand delivers in vector order.
- pg-proto bounds the size of a single DataRow; the 64 MiB limit is tested only after the row is pushed.

### Backend::flush_encrypted_rows

- Every held row has the same column count as the first row.
- result_column_count is non-zero before plaintexts.chunks(result_column_count).
- projection_columns.len() equals the DataRow width and is positional over PostgreSQL's result columns.
- The portal's statement is the statement that produced these rows.
- The portal attached to the operation at flush is the one that caused the rows to be held.
- format_codes has one entry per result column.
- ciphertexts.len() == rows.len() * result_column_count, so chunk n belongs to row n.
- Every ciphertext in the batch, not just the first row's, carries the identifier of its projection column.
- A configured column whose bytes do not deserialise as EQL is an error.
- cipher.decrypt returns one output per record; a shortfall is not possible.
- The ciphertext was produced under the keyset the connection currently has selected.
- The keyset in effect at flush is the keyset intended for the execution that produced the rows.
- The decrypted Plaintext variant matches the type OID advertised for that column in RowDescription.
- The decrypted Plaintext variant is never BigUInt under a Binary format code.
- The operation still has an ExecuteContext after the decrypt await.

### SettingsProtocol::failed

- OperationId order equals frontend message arrival order on the connection, with no reuse or wrap.
- The OperationId supplied is the frontend message PostgreSQL actually failed on.
- Any recorded boundary (Sync, simple Query, FunctionCall) ends PostgreSQL's skip after a failed extended-protocol message.
- Backend responses arrive in frontend-operation order, so nothing later than the failed operation has been acknowledged (prepared_or_bound) or retired (ready) and the live prepared/portals entry under a restored name belongs to the undone operation.
- An ErrorResponse correlated to an unnamed Parse/Bind means PostgreSQL already destroyed the previous unnamed statement/portal, regardless of where in processing it failed, including for the Proxy's substitute failing Bind.
- The first later journal entry with the same name and kind holds, as its previous, the object installed by the mutation being undone.
- No object created after the boundary was derived from an object this rollback removes (later-batch Bound holds a clone of the rolled-back Prepared; Context portal built from the statement at bind time).
- An uncorrelated ErrorResponse (operation None) invalidates no speculative Parse/Bind/Close and starts no skip.

### ConnectionSettings::command_succeeded

- The CommandComplete tag for a rolled-back transaction is exactly the bytes ROLLBACK with no trailing NUL or other suffix. The tag reaches L325 untrimmed, while the same bytes are NUL-stripped before comparison elsewhere in the Backend.
- The command argument is the statement that produced this CommandComplete. Pairing is positional per operation and the tag is never compared with the command kind, so a Commit paired with any non-ROLLBACK tag takes the commit path.
- PostgreSQL reports COMMIT of an aborted transaction with tag ROLLBACK, and that tag is the only signal used; self.failed and the following ReadyForQuery status are not cross-checked against the L325 decision.
- The tag is valid UTF-8; from_utf8_lossy substitutes U+FFFD, which makes L325 false.
- Every savepoint PostgreSQL recognises by name has an entry whose string equals what a later ROLLBACK TO / RELEASE yields. Names are compared by exact String equality after ASCII-lowercasing of unquoted identifiers, with no length truncation.
- When PostgreSQL confirms ROLLBACK TO for a name L338 does not find, leaving settings and failed unchanged is correct.
- savepoints stays bounded because every transaction ends through L328, ready_for_query(Idle) or reset.
- Every transaction end is visible as a Commit/Rollback command, an Idle ReadyForQuery, or DISCARD ALL; commands classified Other have no effect and Begin does not re-baseline.
- DISCARD ALL is never confirmed after earlier settings changes in the same still-open implicit transaction, since DiscardAll drops the baseline and failed.

### Context::settings_execution_succeeded

- The n-th CommandComplete correlated to an operation corresponds to the n-th statement Proxy parsed for it; the popped command kind is never compared with the tag, so a popped DiscardAll resets settings and prunes regardless of the tag.
- tag carries no trailing NUL when compared with "ROLLBACK".
- Every portal that must survive DISCARD ALL has a SettingsProtocol.portals entry.
- PostgreSQL does not roll back a DISCARD ALL it has confirmed; reset() discards the rollback baseline and savepoints.

### SettingsProtocol::succeeded

- The n-th CommandComplete correlated to a simple-query operation completes the n-th statement of forwarded_statements; the pop is positional and the tag is not an argument.
- PostgreSQL splits the forwarded Query text into the same statements sqltk produced, so queue length equals the number of CommandCompletes (EmptyQueryResponse and PortalSuspended pop nothing).
- An extended Execute produces at most one CommandComplete under its id, for the command cloned into bound.prepared.command at Bind time.
- pg-proto correlates each CommandComplete with the Query/Execute operation id and never with a Sync id (a sync-created entry has empty commands and barrier true).
- Every Context portal that must outlive DISCARD ALL has a same-named entry in SettingsProtocol.portals, because the caller prunes Context tables by name membership only.
- Every Context statement that must outlive DISCARD ALL (including the unnamed one) has a same-named entry in SettingsProtocol.prepared.
- A journal mutation with previous None has no metadata that would be wrong to restore after the discard.
- Journal entries keyed at or below `operation` hold nothing that needs restoring or reporting when dropped.
- pending.barrier, computed on the Frontend path from then-confirmed settings (and for a batch OR-ed across all statements under an all-succeed projection), still describes the confirmed command.
- A popped DiscardAll coincides with a CommandComplete tagged DISCARD ALL, so the tag-driven complete_deallocation pass and this command-driven pass act on the same event.

### postgresql::handler (forward loop, capacity retry, outcome drain)

- pg-proto accept enforces the chosen ServerTlsPolicy (rejects non-TLS startup under Required) before authentication
- StaticMd5ServerCredentials rejects wrong password and wrong user before accept returns Session
- ClientTlsPolicy::libpq(VerifyFull) verifies chain and server name; SslMode::Prefer behaves as the L359-L360 comment says, and whether Prefer validates the certificate at all
- A ForwardThenFlush middleware output (frontend.rs:345-346, Execute branch only) is reported as ForwardedMessage::FrontendExpanded so L216 matches
- After Capacity, pg-proto retains the rejected frontend message and forward_frontend retries exactly it, with frontend_operation side effects applied exactly once
- forward_backend never blocks on the client socket and forward_frontend never consumes a backend message
- CommandComplete, ErrorResponse and ReadyForQuery answering tracked operations reach backend_operation with Some(operation) ordered correctly; a None leaves a flag set
- An until_ready barrier entry (simple Query, Sync) is cleared only by ready() with operation id >= its own
- The backend always emits a flag-clearing message or a Copy-in/both response after a flushed Execute, Query or Sync
- No frontend message depending on a pending outcome is intercepted between the Copy early exit and the outcome's arrival
- A frontend handler in wait_for_ddl is always released because the same task keeps reading the backend
- The cancel key is not left registered when accept fails or is dropped by the timeout before the guard exists
- pg-proto checks the CancelRequest secret against the process-global registry before forwarding
- Terminate always surfaces as Forwarded(Terminate)

### SchemaMiddleware::execution_succeeded

- Each terminal message has exactly one queued Execute entry and vice versa; for simple queries the entry count is the sqltk statement count while the completion count is PostgreSQL's.
- A terminal message means the statement's schema effect took place as written; PortalSuspended, EmptyQueryResponse and a CommandComplete with any tag are applied identically.
- A confirmed full ROLLBACK undoes everything the overlay accumulated since the last adoption, so dirty=false is correct afterwards.
- temporary.failed=true set by full rollback is consumed before any later transaction's temporary-table changes are confirmed.
- A bare table name refers to the modelled schema, and a bare name the temporary layer resolves refers to the temporary table.
- The pinned base schema contains every catalog table PostgreSQL resolves for a modelled ALTER TABLE.
- Classification recomputed at completion (split_change L822/L828, models_ddl L788, reading current encrypt_config and base) agrees with the flags stored at prepare time that gate the arm (L778) and the tail (L706, L806).

### Context::ready_for_query

- boundary is the id of the Sync / simple Query / FunctionCall that this ReadyForQuery answers.
- Every operation with id <= boundary has already received all its backend messages, so unconditional removal at mod.rs L984-L993 loses nothing.
- An uncorrelated ReadyForQuery(Idle) is a valid signal to end the settings transaction and clear extended-protocol discard.
- failed == true at Idle refers to a failure inside the transaction whose transaction_start snapshot is held.
- A pending entry with id <= boundary holds no command PostgreSQL executed but has not yet confirmed.
- A journal snapshot surviving readiness does not carry a Context-level portal / owner / suspended execution that this sweep removed.
- The keyset logged for a slow statement is the one in effect while it ran.

### ZeroKms::decrypt_eql

- cipher.decrypt returns exactly one output per submitted record, in submission order; a shorter result leaves trailing Some-input slots as None with Ok returned, and a reordered result places plaintexts in other inputs' slots.
- On the RootOnly (database read) path a stored payload's keyset id matches the connection's keyset.
- cipher.decrypt returns Err for the whole call when any single record fails authentication, instead of omitting or substituting an element.
- The cipher_cache key string is unique per keyset: no identifier renders as "default" or as another identifier's string.
- sv[0] is the entry whose plaintext is the value for the column (root is defined by position, not selector).
- record_with_selector binds the entry ciphertext to the supplied selector through nonce and AAD, so an entry presented under a different selector fails.
- Decrypted root bytes decode to the Plaintext variant the column expects, and non-root entry bytes are not legal Plaintext values.
- decryption_policy needs no structural check because ZeroKMS verifies a policy MAC during key retrieval for tag-version 1 records.
- A cached ScopedCipher remains an authorised scope for the cache TTL.
- Envelope fields outside the encrypted record (identifier, SEM terms, entry term and is_array) are consistent with the plaintext.

### data_row::as_ciphertext / to_eql_ciphertext (with is_json_entry, json_entry_into_ste_vec_document, log_deserialise_error)

- row.columns.len() equals column_configuration.len(), so the output has one entry per row column (the caller re-chunks the flattened output by the first row's width).
- Position i of the DataRow and position i of projection_columns denote the same result column.
- A leading 0x01 means binary jsonb and anything else means text JSON, regardless of the portal's result format code.
- A configured column's value is wire-identical to jsonb (domain over jsonb).
- A root-level "s" key appears only on an eql_v3_json_entry produced by a JSON field access in the statement.
- The selector in root "s" is the selector the statement requested, and the retained h/i/v belong to the same document as the moved c.
- The payload's identifier and keyset are validated somewhere after parsing.
- A None output for a configured, non-NULL column is acceptable to downstream code.
- Column bytes are bounded in size and nesting before the full serde_json::Value parse.

### Backend::check_column_config

- Every buffered row's ciphertext identifiers are checked against the projection columns.
- The first buffered row has at least projection_columns.len() columns, so the examined pairs are aligned.
- projection_columns is positional over, and the same length as, the DataRow columns PostgreSQL returned.
- A None projection entry means the column holds no EQL payload.
- ct.identifier() reflects the column the value was actually stored in / encrypted for.
- The configured identifier at flush time equals the i stored when the value was encrypted.
- The portal's projection_columns belong to the statement that produced the buffered rows.

### Recurring themes among these

- **Correlation and ordering supplied by pg-proto**: operation ids on DataRow, terminal messages, ErrorResponse and ReadyForQuery; monotonic id order; `None` cases.
- **Statement count and identity**: sqltk's split of a Query versus PostgreSQL's completions; positional pops with no tag comparison.
- **Width and position**: projection length versus row width versus format-code count versus cipher output count.
- **What stored bytes are taken to be**: jsonb wire shape, root `"s"`, self-reported identifier, keyset of origin.
- **PostgreSQL protocol behaviour taken as given**: skip-to-Sync extent, unnamed statement / portal destruction, tag text, transaction boundaries, DISCARD ALL in pipelines.
- **Black-box cryptographic guarantees**: one output per record in order, whole-call failure, selector and descriptor binding, keyset scope enforcement.
- **Liveness of the driver loops**: a flag-clearing message always arrives; nothing depends on an outcome across the Copy early exit.

## 9. Assumptions established elsewhere (pointers)

| Assumption | Established by |
| --- | --- |
| `encrypted_rows` non-empty and `encrypted_rows_operation` `Some` on flush entry; all held rows share one operation | Caller `intercept_backend`, `backend.rs` L231, L266-L279, L286 (R02) |
| No Frontend handler runs while a Backend handler is alive, or between lock release and re-acquisition in `ready_for_query` | `middleware/mod.rs` L54-L76 (`&mut self`), driver single task (R05, R07, R09) |
| The handler's Context and both middleware halves observe the same state | `context/mod.rs` L57-L70 (Arc fields); `middleware/mod.rs` L32, L39-L40 (R07) |
| `require_tls` without TLS config never reaches session setup | `driver.rs` L89, L344-L348; `config/tandem.rs` L284-L286 (R07) |
| The keyset argument is `None` whenever a default keyset is configured | `Context::validate_setting`, `context/lifecycle.rs` L521-L528, on the frontend path (R10) |
| A settings command is applied only with a correlated `CommandComplete` | `context/mod.rs` L1573-L1575; `lifecycle.rs` L453-L454 (R05) |
| The protocol-state write lock spans `succeeded`, `command_succeeded` and the DISCARD ALL prunes | `context/mod.rs` L1576-L1601 (R05, R06) |
| A running operation keeps decrypting with its own portal after the portal is removed by name | `ExecuteContext` owns an `Arc<Portal>`, `context/mod.rs` L651-L653, L669-L671, L696 (R05) |
| `Portal::format_codes` is not called on `Passthrough` during a flush | `backend.rs` L482-L488 (R02) |
| `in_flight_ddl` decrement matches its increment | Same predicate at push and every removal site, `schema/middleware.rs` L513-L516, L531-L532, L811, L942-L943, L966-L979 (R08) |
| `self.resolver` is always editable, so `as_schema_with_edits().unwrap()` holds | `schema/middleware.rs` L351, L399, L768 (R08) |
| `apply_encrypt_config` keying by last name component is sound | Caller's `models_ddl` guard, `schema/middleware.rs` L788 (R08) |
| `journal.remove(&id).unwrap()` cannot fail | Keys collected under `&mut self` and removed once, `settings_protocol.rs` L365-L371, L380 (R03) |
| `boundaries` is in ascending order | Append-only push from Frontend paths; ordering itself rests on pg-proto id order (R03) |
| The caller applies `restored` in returned order and filters `released` through reference checks | `context/mod.rs` L221-L265, L324-L335 (R03) |
| With operator-level `mapping_disabled` no Pending carries a command | `frontend.rs` L189-L211; `lifecycle.rs` L42-L70, L307-L310; `settings_protocol.rs` L443-L450 (R05) |
| On Commit / Rollback the baseline is always `Some` | `settings.rs` L318 precedes L326 (R04) |
| `result[idx]` in `decrypt_eql` is in bounds; output length equals input length | `zerokms.rs` L421, L445, L461 (R10) |
| In `AllEntries` mode every payload's keyset id equals the cipher's | `zerokms.rs` L401-L408 (R10) |
| Inbound payload belongs to its destination column | `validate_storage_metadata`, `postgresql/inbound_eql.rs` L98-L114 (R10); read path: `check_column_config`, first row only (R12) |
| `json_entry_into_ste_vec_document` cannot hit its error site | Gate at `data_row.rs` L108, L122 (R11) |
| Portals and `portal_operations` share a key set | `install_portal`, `close`, `close_portal_if_current`, DISCARD ALL retain; not established by `restore_failed_preparation` (R09) |

## 10. Unresolved questions (carried forward in full)

Each item names the record or records that raised it.
Items raised in near-identical words by several records are merged and list all of them.

### 10.1 pg-proto: correlation, ordering, operation ids

1. Which backend messages get `Some(operation)` and to which frontend operation: DataRow, ReadyForQuery, ErrorResponse raised at Sync, asynchronous NoticeResponse / ParameterStatus / NotificationResponse (R01).
2. Whether a terminal message can arrive with `operation == None`, which decides whether `backend.rs` L244-L246 can skip a schema pop the other call sites perform (R08).
3. Cases where a `CommandComplete` arrives with operation `None` (R05).
4. Which `OperationId` (if any) is attached to each `CommandComplete` of a multi-statement Query, after CopyDone, and whether a Sync id can ever carry one (R06).
5. Which operation id is attached to `ReadyForQuery` and when it is `None`: startup readiness, readiness after an async error, a suppressed Sync (R09).
6. Operation-id assignment for `CommandComplete`, `ErrorResponse` and `ReadyForQuery`, including when `backend_operation` receives `None` (R07).
7. Which `OperationId` (or `None`) is attached to an `ErrorResponse` raised during COPY sub-protocol messages, which record no boundary (`frontend.rs` L336-L342), and to asynchronous errors (R03).
8. `OperationId` allocation and `Ord`: strict monotonicity in frontend arrival order over a connection lifetime, wrap, reuse (R03, R06, R09).
9. Whether the number of `CommandComplete` messages per operation can differ from the number of statements `SqlParser::parse_statements` produced (R05).
10. Whether a zero-column DataRow reaches the middleware as `columns: vec![]` (R02).
11. DataRow decoding: maximum column length, and whether a zero-length non-NULL column arrives as `Some(empty)` (R11).
12. `BackendMessage::CommandComplete` decoding: whether the payload includes the trailing NUL (`backend.rs` L764 strips one optionally, `backend.rs` L98 does not; affects `settings.rs` L325) (R04, R05, R06).

### 10.2 pg-proto: driver interaction and middleware outputs

13. What a middleware `Err` does on the backend path inside pg-proto (R01).
14. Handling of `Err` from `backend_operation` when `local_execution_failed` fails at `backend.rs` L238-L239 after `discard_execution` was set at L237 (R02).
15. Whether `frontend_operation` can run between two backend messages of one execution, which decides whether `SchemaMiddleware::before_statement` (`schema/middleware.rs` L438-L446) can change `encrypt_config` between a held row and its flush (R01).
16. The task model during the await at `backend.rs` L522 (R02; see section 6 item 2 for what R05 and R07 establish).
17. Exact semantics of `Suppress(msg)` and the delivery order of `Expand` (R01).
18. `Session::forward_next` direction selection order, which decides what `Frontend::intercept` can observe after the Copy early exit (`driver.rs` L241-L245) and after a DDL-only drain exit before ReadyForQuery (R07).
19. Whether `FrontendMiddlewareOutput::ForwardThenFlush` surfaces as `ForwardedMessage::FrontendExpanded` (`driver.rs` L216) (R07).
20. Capacity handling: whether `frontend_operation` runs before the Capacity check, what is retained, and whether `forward_frontend` re-invokes the middleware (`Frontend::intercept` is not idempotent) (R07).
21. Which frontend messages consume pipeline capacity (CopyData in particular), since the capacity loop L149-L209 has no Copy exit (R07).
22. Ordering between `frontend_operation` and `backend_operation`, and behaviour when `Frontend::intercept` returns `Err` after journaling (e.g. `frontend.rs` L237, L279) (R03).
23. `BackendForwarding::Forwarded` payload (upstream original or middleware replacement) as classified by `backend_needs_frontend` (R07).
24. `driver.rs` L140-L270: any hold on later pipelined Bind / Execute until an earlier Parse of the same name is acknowledged (the visible gate at L220, L258-L259 checks `settings_in_flight` and `schema_ddl_in_flight` only) (R03).
25. Sequencing for whether a FunctionCall's ReadyForQuery can arrive while later Execute entries are queued (no schema `ReadyBoundary` is pushed for it, `lifecycle.rs` L75-L89) (R08).
26. `became_ready` error handling in the driver: the state left when `publish_schema_if_changed` fails after the sweep (`lifecycle.rs` L505-L509) and `schema_ready_for_query` is skipped (R09).
27. What happens to a ReadyForQuery that triggers a failed flush at `backend.rs` L236-L248 and never reaches `became_ready` (R09; R01 records the code path, the client-visible and state consequences are not traced).

### 10.3 pg-proto: accept, TLS, authentication, cancellation

28. `Intermediary::accept`: when the cancel key is registered relative to accept failure or timeout drop (`driver.rs` L115-L120), before the guard at L125 exists (R07).
29. `StaticMd5ServerCredentials` and startup handling: whether the client-supplied user is compared, and which startup parameters (database, options, replication) are passed upstream (R07).
30. `ClientTlsPolicy::libpq`: certificate and hostname checks under `SslMode::Prefer` and its fallback conditions (R07).
31. What the client receives on `Error::ConnectionTimeout` (`driver.rs` L135); `diagnostics::connection_timeout` (`postgresql/diagnostics.rs` L37) has no call site in `driver.rs` or `main.rs` L129-L131 (R07).
32. Whether `Error::ConnectionClosed` and `Error::CancelRequest` (matched at `main.rs` L123-L128) are still constructed on any path reachable from the handler (R07).

### 10.4 PostgreSQL protocol behaviour

33. Handling of a simple Query or FunctionCall received while skipping to Sync, against `settings_protocol.rs` L340-L344 treating their boundaries as ending the skipped range (R03).
34. Parse / Bind failure points relative to dropping the unnamed statement / portal, against the rule at `settings_protocol.rs` L383 and the substitute Bind at `frontend.rs` L1745-L1754 (R03).
35. What the Frontend sends upstream for a refused Execute / Query (`lifecycle.rs` L384-L406) and whether PostgreSQL always answers it with an `ErrorResponse` on the same operation; a stored diagnostic reaches the client only via `finish_execution(Failed)` (`context/mod.rs` L363-L367) (R01).
36. Whether PostgreSQL can confirm DISCARD ALL inside an extended-protocol pipeline after an earlier confirmed SET in the same implicit transaction, given that DiscardAll drops the rollback baseline (`settings.rs` L356, L417) (R04).
37. `PreventInTransactionBlock` behaviour for extended-protocol pipelines: whether a confirmed DISCARD ALL can later be aborted (R05).
38. Whether a transaction boundary can occur upstream with neither a top-level Commit / Rollback statement nor an Idle ReadyForQuery (for example transaction control inside CALL in a pipeline), leaving `transaction_start` stale (R04).
39. PostgreSQL identifier normalisation (length limit, non-ASCII folding) against `postgres_identifier` (`proxy/schema/middleware.rs` L1037-L1043) for savepoint names compared at `settings.rs` L338 and L347 (R04).
40. Whether a non-`until_ready` barrier Pending entry can end with `EmptyQueryResponse` or `PortalSuspended`, which skip `settings_execution_succeeded` (`lifecycle.rs` L453-L455) (R07).

### 10.5 SQL parsing (sqltk) and statement counting

41. How sqltk parses COMMIT PREPARED, ROLLBACK PREPARED, PREPARE TRANSACTION, END, ABORT and COMMIT / ROLLBACK AND CHAIN: which map to `Statement::Commit` / `Statement::Rollback` (`settings.rs` L173-L180) and which fall to Other (R04).
42. Which spellings (ABORT, END, ROLLBACK PREPARED, PREPARE TRANSACTION) map to `Statement::Rollback { savepoint: None }`, Commit, Savepoint, ReleaseSavepoint for the schema arms (R08).
43. Whether sqltk's statement splitting of a simple-query string can yield a different statement count or order than PostgreSQL produces `CommandComplete` messages for (positional pairing at `settings_protocol.rs` L289), including statements that end with `EmptyQueryResponse` or COPY responses (R04).
44. `SqlParser::parse_statements`: how sqltk's statement count for a simple Query relates to the number of terminal messages PostgreSQL emits for the same text (R08).
45. `Parser::parse_statements` (`postgresql/parser.rs` L72-L85): whether an empty statement or trailing comment yields a `Statement`, i.e. whether an `EmptyQueryResponse` position has a queued command (R06).

### 10.6 Frontend paths that feed Backend state

46. `lifecycle.rs` L345-L363: whether `Context::close` creates an operations entry (decides the outcome of an `ErrorResponse` correlated to a Close) (R01).
47. `frontend.rs` L476-L531: which statement's projection the single unnamed portal holds in a multi-statement simple Query, since `data_row_handler` and `row_description_handler` consult that one portal for every statement's rows (R01).
48. `set_simple_query_execute_until_ready` (`context/mod.rs` L657) and `install_portal`: confirm a simple Query's `ExecuteContext.portal` is the single portal registered at `lifecycle.rs` L377 for every result set (R02).
49. `Frontend::route` / `Route::Map` construction: which statements of a multi-statement simple Query overwrite the portal at `frontend.rs` L531 (R02).
50. `Statement::has_projection` and `frontend.rs` L1049-L1050: whether an Encrypted portal can coexist with zero-column rows or with more result columns than `projection_columns` (`backend.rs` L491-L508, L547) (R01).
51. Frontend COPY handling: how CopyOutResponse / CopyData on tables with encrypted columns reach the default arm (`backend.rs` L362-L368) (R01).
52. Frontend handling of `SET CIPHERSTASH.*`: whether the statement is forwarded to PostgreSQL unchanged, so that the `CommandComplete` relied on is PostgreSQL's verdict on the same statement (R04).
53. Every frontend path that admits `SET CIPHERSTASH.KEYSET_*`: confirm `validate_setting` (`lifecycle.rs` L521-L528) runs on each, since it alone keeps `keyset_id` `None` when a default keyset is configured (R10).
54. Frontend callers of `may_change_effective`: how a projected false (`settings.rs` L190-L195 for Savepoint / Release) interacts with a later RollbackTo projected against confirmed settings that do not yet contain the in-flight savepoint (R04).
55. `SettingsCommand::may_change_effective` (`context/settings.rs`) and `Intent::changes_schema` (`proxy/schema/middleware.rs`): statements with settings or schema effects that neither predicate flags (R07).
56. `Frontend::parse_handler` / `bind_handler` under session-level `unsafe_disable_mapping`: confirm journal entries are still written (R03).
57. Locally refused Parse / Bind / Close paths in `frontend.rs`: journal mutations that never receive an acknowledgement and can therefore still be keyed at or below a DISCARD ALL operation at `settings_protocol.rs` L302 (R06).
58. `Frontend::parse_handler` early returns and callers of `PreparedStatements::set_statement` / `set_deallocation` (`prepared_statements.rs` L57-L83, `or_default` creation): a Context statement entry with no `SettingsProtocol.prepared` counterpart (R06).
59. The `?` exits of `parse_handler` and `bind_handler`, and pg-proto's handling of handler futures: a `Preparation` left set while its metrics scope is otherwise unreferenced (R05).
60. Frontend exits between `begin_bind` (`lifecycle.rs` L231-L259) and `finish_bind` / `refuse_bind` (L261-L304): whether an operation reaches readiness with `Preparation::Bind { metrics: Some }` still set, leaving that scope outside candidates (R09).
61. `bind_settings_statement`, `Frontend::bind_handler` and `extended_parse_error` (`frontend.rs` L272): whether a Bind to a statement whose Parse was refused after `lifecycle.rs` L153 is refused before `bind_schema_statement` (L238) (R08).
62. `middleware/frontend.rs` call sites of `discarding_extended_messages` (`context/mod.rs` L1550-L1557): dependence on the flag being cleared by an uncorrelated readiness (R09).
63. `unmapped_*` callers in `middleware/frontend.rs`: whether mapping-disabled paths (`lifecycle.rs` L307-L310, L323-L334) leave `SettingsProtocol.portals` and Context portals with different key sets at readiness (R09).
64. `get_portal` callers in `middleware/frontend.rs` (`describe_handler`): by-name reads of `portals[""]` after an earlier DISCARD ALL removed a later pipelined Query's entry (R05).

### 10.7 Context tables, journal and settings protocol internals

65. `PreparedStatements::deallocation_names` and `deallocate` (called at `context/mod.rs` L891-L897): which names an AllStatements deallocation removes relative to the operation id (R01).
66. `context/mod.rs` L1195 onward: the remainder of `close_portal_if_current` (R01).
67. `PreparedStatements::remove` / `set_*`: confirm the caller's remove-then-reinstall (`context/mod.rs` L223-L239) fully replaces the per-name record (R03).
68. Test comment at `settings_protocol.rs` L811-L812 says FunctionCall / config-disabled Query record no boundary while `lifecycle.rs` L58-L63, L75-L89 record one; which is the intended contract; git history of `readiness_boundary` (R03).
69. `SettingsProtocol::failed` / `ready` (`settings_protocol.rs` L333-L364, L453-L482): whether a Pending entry with `until_ready` can keep supplying commands to later `CommandComplete` messages of the same operation after its queue was expected to be exhausted (R04).
70. `SchemaMiddleware::execution_succeeded` (`proxy/schema/middleware.rs` L785, L896): whether its own prepared / portals maps are pruned on DISCARD ALL (R05).
71. `set_execute_for_portal` (`context/mod.rs` L676-L706) and `ready_for_query` (L967-L981): handling of a `suspended_executions` entry whose portal was removed at `context/mod.rs` L1595 (R05).
72. Readers of `suspended_executions` between DISCARD ALL (not pruned at `context/mod.rs` L1587-L1598) and Idle readiness (pruned at L967-L981) (R06).
73. `Backend::data_row_handler` and `Context::get_portal` call sites: whether a later pipelined operation reads its portal from the Context table (pruned at `context/mod.rs` L1595) or from its `ExecuteContext` clone (L669-L670, L682-L696) (R06).
74. Trace `SettingsProtocol::failed` (L365-L430) into `restore_failed_preparation` (`context/mod.rs` L252-L261) for a pipelined Bind following a simple Query whose Idle readiness arrives in between: whether a surviving journal snapshot with `previous == Portal(None)` reinstates a portal the sweep removed (R09).

### 10.8 Schema model

75. Tests for ROLLBACK AND CHAIN and for multi-transaction simple-query messages, where `temporary.failed` (L776) and `dirty = false` (L774) persist across a transaction start inside one readiness window (R08).
76. Intended use of `Intent.modelled` (`schema/middleware.rs` L88, L466), which has no reader in the file (R08).
77. `SchemaManager` load code: how `Schema.name` and multi-schema tables are populated (eql-mapper `schema.rs` L177-L179 resolves one-part names in any loaded schema) (R08).
78. The tokio version in `Cargo.lock` for `Notify::notify_waiters` coverage of created-but-unpolled `Notified` futures (R08).
79. `SchemaMiddleware::apply_ddl` callers of `EncryptConfig::rename_column` / `rename_table` (`proxy/encrypt_config/config.rs` L64-L86): whether stored payloads' `i` is expected to change after a rename (R12).

### 10.9 eql-mapper and projection

80. `projection.columns()` (wildcards, RETURNING, set operations, schema drift after Parse): cases where its length differs from PostgreSQL's result width, the input condition for the `data_row.rs` L21 truncation and for the non-aligned zip and `(None, Some)` arm of `check_column_config` (R02, R11, R12).
81. `ColumnMapper::get_column` (`column_mapper.rs` L52): which `EqlTerm` variants yield `Some(Column)` for a projection column, and whether a JsonAccessor result (advertised TEXT, `context/column.rs` L81) arrives in a form `data_row.rs` L99-L103 handles (R11).
82. `table_column()` name resolution (aliases, schema-qualified, quoted / mixed-case names, views): whether the identifier built at `column_mapper.rs` L42-L44 always equals the `i` written at encrypt time (R12).

### 10.10 cipherstash-client, ZeroKMS and stored payloads

83. `ScopedCipher::decrypt`: whether it always returns one element per input record in input order, and whether one failing record makes the whole call `Err` (R02, R10).
84. `ScopedCipher::decrypt` and its use of `Decryptable::keyset_id`: whether a record whose keyset id differs from the cipher scope is refused, since `zerokms.rs` L401-L408 does not run for RootOnly (R10).
85. `ScopedCipher::decrypt`: whether a record is bound to its identifier, descriptor and keyset (R02).
86. `ScopedCipher::decrypt` with `DecryptOptions::default()`: whether the record descriptor (table / column, `inbound_eql.rs` L107-L111) is authenticated at decrypt and compared with anything, since the Proxy passes no expected descriptor on the read path (`zerokms.rs` L424, L433-L435, L449) (R12).
87. `KeyHeader::record_with_selector` and `RecordWithNonce::{nonce_override, aad_selector}`: confirm the selector is bound into nonce and AAD, and whether the header's descriptor and keyset_id are covered by an entry's AEAD tag; what binds an entry ciphertext to the `h` header, selector `s`, identifier `i` and keyset, all taken from stored bytes without comparison (R10, R11).
88. `EncryptedRecord`: confirm descriptor and keyset_id are authenticated fields of a scalar record (claimed by comment at `postgresql/inbound_eql.rs` L104-L106) (R10).
89. `Plaintext::from_slice`: which byte strings it accepts, whether non-root SteVec entry plaintexts are rejected (comment at `zerokms.rs` L459-L460), and what it yields for an extracted `eql_v3_json_entry` reshaped at `data_row.rs` L133-L151 (R10).
90. `Plaintext::from_slice` and `postgres_types` `ToSql` for `Option<T>`: what `binary_to_sql` emits for a variant with inner `None` (`to_sql.rs` L40-L55) (R02).
91. `IdentifiedBy` Display impl: whether a keyset name renders as the bare name and so can share a `cipher_cache` key with `"default"` or with a UUID identifier (`zerokms.rs` L147-L152, `proxy/mod.rs` L75) (R10).
92. `ScopedCipher::init` with `identified_by = None`: the scope of the `"default"` cache entry created at `proxy/mod.rs` L75 when no default keyset is configured (R10).
93. `ScopedCipher`: whether a cached cipher re-checks access with ZeroKMS per decrypt or holds its grant for the cache TTL (`zerokms.rs` L131) (R10).
94. The ZeroKMS key-retrieval path: confirm the decryption_policy MAC claim in the comment at `zerokms.rs` L410-L412 and which tag versions it covers (R10).
95. `EqlCiphertextV3` / `EncryptedPayloadV3` / `SteVecPayloadV3` serde attributes: how the Encrypted and SteVec variants are discriminated, and what an object with root `c` but no `s`, or with both `sv` and `c`, deserialises to (R11).
96. The same types for `deny_unknown_fields`, since after the reshape scalar index keys (`bf`, `hm`) remain at the root of a SteVec-shaped object (R11).
97. The same types: whether `v` is validated, whether `h` is required on a SteVec document, and whether `i` is constrained (R11).
98. `eql::Identifier` (PartialEq impl and field set): exactly what `backend.rs` L577 compares (R12).
99. `EqlCiphertextV3::identifier()` for both variants: confirm it returns the deserialised `i` field and cannot panic (R12).
100. `Backend::check_column_config` and `as_ciphertext` alignment across rows (`backend.rs` L569-L600): confirm the first-row-only identifier coverage recorded in R02, since `decrypt_eql` performs no identifier check (R10; R12 has since analysed this function, see section 6 item 7).
101. `packages/cipherstash-proxy-integration/src/select/jsonb_get_field_as_ciphertext.rs`: which root-`s` payload shapes are exercised end to end; the unit tests at `data_row.rs` L229-L307 contain none (R11).

### 10.11 serde_json, diagnostics and logging

102. serde_json 1.0.151 and its enabled Cargo features (`unbounded_depth`, `arbitrary_precision`, `preserve_order`): recursion limit, duplicate-key behaviour and trailing-data handling in `from_slice` (R11).
103. serde_json and cipherstash-client error Display output: whether the string logged at `data_row.rs` L154 can contain fragments of the stored column value (R11).
104. `diagnostics::system_error`: whether table / column strings taken from stored bytes at `backend.rs` L592-L593 are length-limited or sanitised before entering the `ErrorResponse` (R12).

### 10.12 Process and scope

105. `ANALYSIS_FORMAT.md` was not found on disk; R08's prose follows the section order of R06 (R08).
106. `proxy/schema/manager.rs`, `proxy/schema/eql_domains.rs` and `proxy/encrypt_config` were not analysed line by line in this pass; statements about them here are from orientation.
107. pg-proto 0.12.0 and cipherstash-client 0.42.2 sources are not on disk; every item in 10.1 to 10.3 and most of 10.10 can be closed only by obtaining them.
