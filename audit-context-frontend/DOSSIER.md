# Audit context dossier: packages/cipherstash-proxy

This is a system model built from twelve per-function records under `audit-context/functions/`.
It describes structure, invariants and assumptions.
It names no defects, proposes no changes and assigns no severity.
An unenforced assumption is recorded as an unenforced assumption.

Record tags used throughout:

| Tag | Function | Prose file |
| --- | --- | --- |
| R01 | `Frontend::route` | `functions/01-Frontend-route.md` |
| R02 | `Frontend::query_handler` | `functions/02-Frontend-query_handler.md` |
| R03 | `Frontend::parse_handler` | `functions/03-Frontend-parse_handler.md` |
| R04 | `Frontend::bind_handler` | `functions/04-Frontend-bind_handler.md` |
| R05 | `Frontend::intercept` | `functions/05-Frontend-intercept.md` |
| R06 | `Frontend::encrypt_params` | `functions/06-Frontend-encrypt_params.md` |
| R07 | `Frontend::merge_inbound_eql` | `functions/07-Frontend-merge_inbound_eql.md` |
| R08 | `inbound_eql::parse` | `functions/08-inbound_eql-parse.md` |
| R09 | `sem_terms_match` | `functions/09-sem_terms_match.md` |
| R10 | `check_copy_into` | `functions/10-check_copy_into.md` |
| R11 | `Frontend::encrypt_literals` | `functions/11-Frontend-encrypt_literals.md` |
| R12 | `Frontend::transform_statement` | `functions/12-Frontend-transform_statement.md` |

Paths are relative to `packages/cipherstash-proxy/src/` unless they start with `eql-mapper/`.

## 1. Coverage and limits of this model

All twelve records sit on the client-to-database half of the pipeline: `postgresql/middleware/frontend.rs` and `postgresql/inbound_eql.rs`.
Everything else in the orientation is known here only through what those records observed at call sites.
The following were not analysed function by function, and statements about them below are second-hand:

- `postgresql/middleware/backend.rs` (ErrorResponse replacement, RowDescription rewriting, DataRow buffering and decryption, ReadyForQuery transitions).
- `postgresql/driver.rs` (forward loop, outcome drain, cancellation registry, TLS policy, credentials).
- `postgresql/context/` internals beyond the facade calls the Frontend makes (`settings.rs`, `settings_protocol.rs`, `lifecycle.rs`, `prepared_statements.rs`, `portal.rs`).
- `proxy/schema/` (`manager.rs`, `middleware.rs`, `eql_domains.rs`) and `proxy/encrypt_config/`.
- `proxy/zerokms/` beyond the encrypt and inbound-decrypt paths; the result-row decrypt path was not read.
- `postgresql/rewrite/data_row.rs`, `postgresql/data/to_sql.rs`.
- `main.rs`, `config/`, `connect/`, `tls/`, `cli/migrate`, `prometheus.rs`, `log/`.
- External crates: pg-proto 0.12.0 and cipherstash-client 0.42.2 (source not located on disk), sqltk 0.11.0 and sqltk-parser 0.56.0-cipherstash.3 (not read), eql-mapper (read in part).

Several records note that the format resources `ANALYSIS_FORMAT.md`, `FUNCTION_MICRO_ANALYSIS_EXAMPLE.md` and `DOMAIN_NOTES.md` were not found, so each prose file uses its own layout (R01, R02, R03, R06, R11).

## 2. Actors and what they reach

| Actor | Trust | Entrypoint | Reaches |
| --- | --- | --- | --- |
| Unauthenticated network peer | untrusted | `main` accept loop, then `postgresql::handler` (`postgresql/driver.rs`) | pg-proto startup, SSLRequest/TLS policy, MD5 authentication against the shared database credentials; one handler task and one connection number per accepted socket. The middleware is not reached before authentication. |
| Unauthenticated network peer | untrusted | CancelRequest | Lookup in the process-global `CANCELLATION_REGISTRY` and a cancel forwarded upstream. |
| Unauthenticated network peer | untrusted | `prometheus::start` HTTP listener on `server.host:prometheus.port` | Read-only metrics; no authentication in this crate. |
| Authenticated client application | semi-trusted | `Frontend::intercept`, once per frontend message | Per-connection protocol state, SchemaMiddleware state, SettingsProtocol, the process-wide ZeroKms cipher cache, ZeroKMS encrypt/decrypt calls, and SQL forwarded or rewritten to PostgreSQL. |
| Authenticated client application | semi-trusted | `SET`/`RESET CIPHERSTASH.*`, `DISCARD ALL` in-band | `ConnectionSettings` (effective keyset, `unsafe_disable_mapping`), committed only after PostgreSQL confirms; the keyset selects the cipher-cache key and ZeroKMS scope. |
| Authenticated client application | semi-trusted | Inbound EQL payloads in literals and Bind parameters | `inbound_eql::parse`, then `merge_inbound_eql`, ZeroKMS decrypt and re-encrypt, and finally the statement text or Bind sent upstream. |
| Authenticated client application | semi-trusted | DDL in forwarded statements | SchemaMiddleware overlay and, on commit, `SchemaManager::reload` and a new process-wide `CommittedSchemaSnapshot` seen by every connection. |
| Connection pooler | semi-trusted | Same as the client application | Shares one Proxy connection across tenants, so the scope of effective connection settings is the boundary between them. |
| Upstream PostgreSQL | semi-trusted | `Backend::intercept`, once per backend message | Protocol-state transitions, settings commit or rollback, schema execution outcomes, row decryption, ErrorResponse replacement. |
| Upstream PostgreSQL | semi-trusted | Catalog content read by `SchemaManager::init`/`reload` | The `(Schema, EncryptConfig)` pair that every routing decision reads. |
| Other database writers and other Proxy connections | semi-trusted | Catalog changes and stored rows | The committed schema snapshot this connection adopts and the EQL payloads it later decrypts. |
| ZeroKMS | trusted | Outbound calls from `ZeroKms::init_cipher`, `encrypt`, `decrypt` | Ciphertexts, SEM terms and plaintext returned to the Frontend; only the result count is checked by this crate. |
| Operator | trusted | `TandemConfig::load` (file, `CS_*`, `DATABASE_URL`, CLI), signals, `cli::run`/`Migrate::run` | All configuration, including `development.disable_mapping` and `development.enable_mapping_errors`; SIGHUP replaces the `Proxy` held by the accept loop; migrate connects directly to the database. |
| eql-mapper, pg-proto | trusted | Library calls | Statement classification, type checking, rewriting; framing, ordering, authentication, backpressure, operation ids. |

## 3. State: who reads, who writes, and what that implies

### 3.1 `ConnectionProtocolState` (per connection, `Arc<RwLock>`, `postgresql/context/mod.rs`)

Shared by Frontend and Backend through Arcs on one `CipherStashMiddleware` (R05, `middleware/mod.rs` L39-L40).

| Field | Frontend writes observed | Frontend reads observed |
| --- | --- | --- |
| `prepared_statements[name]` | `begin_parse` removes and recreates the entry with `statement: None` (R03, `lifecycle.rs` L103-L112); `finish_parse` installs `Arc<Statement>` only for `Route::Map` (R03); `refuse_parse` removes it (R03, R05); `set_statement_deallocation` (R03); `close` removes it ahead of acknowledgement with a journal snapshot (R05). | `begin_bind` snapshot and `is_prepared` (R04); `describe_handler` (R05). |
| `operations[op].preparation` | `begin_parse` sets `Parse{intent:false}`, `prepare_parse` sets `intent=true`, `finish_parse` consumes it (R03); `begin_bind` sets `Bind`, `finish_bind` consumes it (R04). | `finish_parse` requires `intent=true`; `refuse_parse` requires a Parse preparation; `finish_bind` requires `Bind` (R03, R04, R05). |
| `operations[op].deallocations` | `set_deallocations` at `frontend.rs` L421-L427, before later error exits of `query_handler` and not undone by `refuse_query` (R02). | Backend, at completion (not read). |
| `operations[op].error_response` | `refuse_query`, `refuse_parse`, `refuse_bind`, `refuse_execute` store the diagnostic before the substitute message is assigned (R05). | Backend replaces PostgreSQL's ErrorResponse with it (R05, `backend.rs` L315-L321, not read in full). |
| `portals`, `portal_operations`, `suspended_executions` | `register_query` installs the unnamed portal (R02); `finish_bind`/`install_portal` replaces portal, owner and suspended execution atomically (R04); `refuse_query`/`refuse_execute` attach an `ExecuteContext` under the unnamed portal name whatever the client named (R05); `execute` installs an `ExecuteContext` with `portal: None` for an unknown name (R05). | Backend reads one portal per operation (R02, not read). |
| `settings` | Not written by any Frontend function analysed; `route` is explicitly getter-only (R01). | `unsafe_disable_mapping` in `route` (R01) and `begin_bind` (R04); keyset in each `Context::encrypt` and `Context::decrypt_inbound_eql` call (R06, R07, R11); `EffectiveSettings` snapshot in `prepare_parse` and `bind_settings_statement` (R03, R04). |
| `statement_metrics` | `start_metrics_scope`, `update_statement_metadata`, `record_parse_duration`, `with_metrics_scope`, `add_encrypt_duration` (R02, R03, R04, R06, R11). | Diagnostics only; an unknown scope id is a silent no-op (R02, R11). |

Implied invariant: a `prepared_statements` entry with `statement: Some` exists only for a Parse that `route` mapped and `finish_parse` completed.
An entry with `statement: None` records only that a Parse was routed Forward; the Forward reason is not stored (R03).
`is_prepared(name)` is key presence only and is true for both (R04).

### 3.2 `SettingsProtocol` (inside protocol state, `postgresql/context/settings_protocol.rs`)

| Field | Writes observed | Reads observed |
| --- | --- | --- |
| `journal[op]`, `prepared[name]` | `prepare_parse` captures `EffectiveSettings` and `needs_mapping = requires_schema(statement)` before routing (R03); `finish_parse` sets `literal_sensitive` (R03). | `bind` compares the stored snapshot with current settings, only when `prepared[name]` exists and `needs_mapping` is true; keyset mismatch refuses only when `literal_sensitive` (R04, L210-L220). |
| `portals` | `bind` journals and removes the previous portal before any check, then records `Bound{settings, encrypted_values}` (R04, L200-L229). | `execute` compares settings only when the portal exists and `needs_mapping`; an unknown portal yields `Ok(false)` (R05, L239-L249). |
| `pending[op]` | `simple_query` via `register_query` (R02). | Backend, per completion (not read). |
| `discarding_until_sync` | Set only on Backend's failure path (L345); cleared by `sync` (L440) and at readiness (L457) (R05). | `Frontend::intercept` discard gate (R05). |

### 3.3 `Frontend.failed_extended_batch` (Frontend-private)

Set on every local refusal of Execute, Parse and Bind (`frontend.rs` L245, L271, L296, L306, L317) and nowhere else.
Cleared only by a Sync reaching the discard gate (L181), before `context.sync` is attempted.
Not visible to Backend and not cleared by ReadyForQuery (R05).

### 3.4 `SchemaMiddleware` (per connection, `proxy/schema/middleware.rs`)

| Field | Writes observed from Frontend paths | Reads observed |
| --- | --- | --- |
| `base`, `encrypt_config`, `resolver` | `adopt_latest` via `prepare_schema_for_statement`/`prepare_parse`, only when idle, clean and no DDL in flight; written sequentially (R02, R03, L392-L402, L438-L446). | Three separate locks: `has_encrypted_columns` reads `encrypt_config` and `base`; `encrypt_config()` for column lookup; the resolver argument for type checking and `check_copy_into` (R01, R03, R10). |
| `prepared[name]` | `prepare_parse` writes the DDL intent before later steps of `prepare_parse` can fail; `refuse_parse` does not touch it (R03). | Later Bind/Execute (not read). |
| `portals[portal]` | `begin_bind` inserts or removes unconditionally, no journal; `refuse_bind` does not touch it (R04). | `execute` (R05). |
| `executions`, `in_flight_ddl`, `transaction_active` | `simple_query` via `register_query` pushes one entry per statement plus a `ReadyBoundary` (R02); `sync` pushes a `ReadyBoundary`; `unmapped_sync` does not (R05). | `wait_for_schema_execution` waits for `in_flight_ddl == 0` with no timeout (R02). |
| `temporary`, `unmodelled`, `local_unmodelled` | Backend-driven (not read). | `ensure_schema_modelled`, called only when a statement `requires_schema` (R02, R03); `batch_resolvers` skips unmodelled catalog changes silently (R02). |

### 3.5 Process-wide state

| State | Writers | Readers observed |
| --- | --- | --- |
| `CommittedSchemaStore` | `SchemaManager::init_reloader`, `reload`, `publish_if_newer`, `mark_publication_pending`, `publication_succeeded`. | `prepare_schema_for_statement` publishes pending shared schema (awaits reload, no timeout) and adopts the latest snapshot (R02, R03). |
| `ZeroKms.cipher_cache` | `ZeroKms::init_cipher`, keyed by keyset identifier string or `"default"`, TTL and size bounded. | Every encrypt and inbound decrypt (R04, R06, R07, R11). The key is client-selectable through `SET CIPHERSTASH.KEYSET_*`. |
| `CANCELLATION_REGISTRY` | pg-proto on session accept; `CancellationRegistrationGuard::drop`. | CancelRequest forwarding (not analysed). |
| `Proxy` | `init`, `reload_application_config` on SIGHUP. | Each new `Context` holds an `Arc<TandemConfig>` fixed at construction, so operator `mapping_disabled` is constant for a connection's life (R05). |
| Prometheus recorder | `counter!`/`histogram!` call sites. | The Bind path gates on `prometheus_enabled` (`frontend.rs` L1143-L1147); the literal path does not (L671, L706-L708) (R06, R11). |

### 3.6 Message-local values that carry meaning across functions

- `Statement` (stored at Parse): `param_columns`, `projection_columns`, `literal_columns`, `output_params`, and `postgres_param_types` (client OIDs cast `u32 as i32`, unvalidated). It carries no schema version and no keyset (R03).
- `Route` (`Map` or `Forward(reason)`): computed once per simple-Query statement and once per Parse, never at Bind or Execute (R01).
- `InboundEql` (`Store` or `Query`): produced by `inbound_eql::parse`, consumed by `merge_inbound_eql`. The enum is directly constructible; no type-level proof that `parse` ran (R07, R08).
- The positional vectors `literals`, `literal_columns`, `inbound`, `skip`, `plaintexts`, `encrypted` (section 5.1).

## 4. End-to-end flows

### 4.1 Gate order for every frontend message (`Frontend::intercept`, R05)

1. `Terminate` is forwarded before any state is read.
2. If `failed_extended_batch` or `discarding_until_sync`: everything except `Sync` is suppressed, including `Query` and `FunctionCall`; `Sync` clears the Frontend flag and runs `context.sync`.
3. If operator `mapping_disabled`: the unmodified message is forwarded, no SQL is parsed, and only correlation bookkeeping runs (`unmapped_*`).
4. Otherwise the per-message handler runs.

The operator switch (step 3) and the connection setting `unsafe_disable_mapping` are distinct.
The connection setting is consulted later, inside `route` (`frontend.rs` L1254) and `begin_bind` (`lifecycle.rs` L239) (R01, R04, R05).

### 4.2 Simple Query (R02, R01, R11, R12)

1. `from_utf8_lossy` of the query bytes, then `SqlParser::parse_statements` (RESET forms become SET sentinels, `SET ALL` rejected, split on `;` tokens when a `COPY ... FROM STDIN` is present).
2. Per statement: `validate_setting`.
3. `settings_simple_query_requires_fail_closed`: refuse if a `requires_schema` statement follows a possibly settings-changing statement.
4. `set_deallocations` on the operation.
5. `prepare_schema_for_statement` (publish pending, adopt latest when idle and clean).
6. `simple_query_requires_fail_closed`: refuse if a `requires_schema` statement follows DDL classified as encryption-changing.
7. If any statement `requires_schema`: `wait_for_schema_execution`, then `ensure_schema_modelled`.
8. `batch_table_resolvers`: one resolver per statement with earlier encryption-neutral modelled DDL applied to scratch copies.
9. `route` every statement before any literal is encrypted.
10. Batch rules: a Map route together with any `Statement::Copy` is refused; a Map route together with a `Forward(Fallback)` is refused with the first fallback's error.
11. For each Map route with `requires_transform`: `encrypt_literals` then `transform_statement`.
12. `register_query`: unnamed portal (the `Statement` of the last Map route, or passthrough), execute-until-ready context, schema execution queue entries plus `ReadyBoundary`, settings `Pending`. Four sequential fallible steps, not atomic.
13. If at least one statement was transformed, every statement is re-rendered from its AST and joined with `;`; otherwise the client's original bytes are forwarded.

On any `Err`, `intercept` calls `refuse_query` and forwards a `DO`/`RAISE` block built from diagnostic fields through `quote_literal`; none of the client's SQL is forwarded (R05).

### 4.3 Extended protocol: Parse (R03, R01, R11, R12)

1. `begin_parse`: journal the prior statement, remove it, create a fresh entry with `statement: None`, set `Preparation::Parse{intent:false}`.
2. `from_utf8_lossy` and `SqlParser::parse_statement` (first statement only in the COPY branch).
3. `set_statement_deallocation`.
4. `prepare_parse`: adopt schema, `ensure_schema_modelled` when `requires_schema`, write schema intent `prepared[name]`, `validate_setting`, capture `EffectiveSettings`, set `intent=true`.
5. `get_table_resolver`, then `route` with the client's parameter OIDs.
6. On `Route::Map` with `requires_transform`: `encrypt_literals`, `transform_statement`, `output_params_from_plan`; `message.query` is rewritten only when the transform returned `Some`.
7. On `Route::Map`: `message.parameter_types` rewritten by `rewrite_parse_param_types` (empty if the client declared none; otherwise JSONB or TEXT for encrypted outputs, client OID or 0 for native ones).
8. `finish_parse(Some(statement))` for Map, `finish_parse(None)` for Forward.

On `Err`, `intercept` calls `refuse_parse`, sets `failed_extended_batch`, and forwards a substitute Parse that keeps the client's statement name, replaces the SQL with a cast to a nonexistent `__cipherstash_<uuid>` type and clears the parameter types (R05).

### 4.4 Extended protocol: Bind (R04, R06, R07, R08, R09)

1. `Bind::try_from(pg_proto::Bind)`; a format-code count that is not 0, 1 or the parameter count leaves `intercept` as a connection-level `Err` without `refuse_bind` (R05).
2. `begin_bind`: `SettingsProtocol::bind` (journal, drift check), `SchemaMiddleware::bind`, preparation set to `Bind`; returns the statement snapshot and the connection's mapping-disabled flag.
3. If mapping is disabled for the connection: install `Passthrough(None)` and return before `is_prepared` and without consulting the stored statement.
4. If the name has no `prepared_statements` entry: refuse.
5. If a `Statement` is stored and `has_params()`: `encrypt_params` (`Bind::inbound_eql` then `to_plaintext_skipping` then `Context::encrypt` then `merge_inbound_eql` then `project_query_operand`), then `Bind::rewrite`.
6. Portal is `Encrypted` only if `projection_columns` is non-empty, with the client's result format codes copied unvalidated; otherwise `Passthrough`.
7. `finish_bind` is the last fallible call.

On `Err`, the client's Bind is forwarded with only the statement name replaced by a random name; portal name, parameter formats, parameter values and result formats are retained (R04, R05).

### 4.5 Extended protocol: Describe, Execute, Close, Sync (R05)

- Describe records the stored `Statement` (or `None`) for Backend's description rewriting.
- Execute has no portal-existence check; refusal comes only from `SettingsProtocol::execute`'s settings comparison, before any schema `PendingExecution` or portal-derived `ExecuteContext`. `ForwardThenFlush` is returned only from this arm, when a settings barrier or DDL is reported or the Execute is refused.
- Close removes local metadata before acknowledgement, with a journal snapshot for rollback.
- Sync pushes the schema `ReadyBoundary` and `SettingsProtocol::sync`.
- `FunctionCall`, COPY data, Flush and every other variant fall to the default arm and are forwarded.

### 4.6 Local refusal round trip (R02, R03, R04, R05)

Handler `Err` leads to `refuse_*` storing the diagnostic in `operations[op]`, a substitute message sent upstream, PostgreSQL answering with an ErrorResponse, pg-proto correlating that response to the same operation id, and Backend replacing it with the stored diagnostic.
If the substitute instead completes, `finish_execution(Completed)` drops the stored diagnostic (`context/mod.rs` L363-L372).
The rollback of local state for a refused Parse or Bind depends on Backend calling `postgres_failed` and replaying the journal (`context/mod.rs` L216-L261); neither was read.

### 4.7 Inbound EQL (R08, R07, R09, R11, R06)

1. Detection is by content: a 32-character lowercase-hex value at a `JsonAccessor`/`JsonPath` query-operand position is a `Selector`; a JSON object with `v`, `i` and one of `c`/`h`/`sv` is storage-shaped; a JSON object with a query-shaped key set is query-shaped; anything else is plaintext.
2. Storage shape is tested first and does not read `query_operand` or `eql_term`.
3. `Store` passes `validate_storage_metadata` (version, identifier, descriptor string, term presence or non-empty `ste_vec`).
4. `Query` passes `validate_query_metadata` (structure only; version and identifier on the `Encrypted` arm only).
5. `merge_inbound_eql` writes `Query` payloads straight to the output slot, and `Store` payloads only after `decrypt_inbound_eql` succeeds (keyset id equal to the cipher's, every SteVec entry submitted), `Context::encrypt` of the recovered plaintext succeeds, the derived value is `EqlOutput::Store`, and `sem_terms_match` is true.
6. The value kept is the client's payload as deserialised, not the derived one.
7. `project_query_operand` converts `Store` to `Query` at query-operand positions via `into_query_operand`.
8. The result is embedded in SQL text by `transform_statement` (literals) or written into the Bind by `Bind::rewrite` (parameters).

### 4.8 Settings change (R02, R03, R04, R05)

A `SET CIPHERSTASH.*` is validated at the Frontend, forwarded, and committed to `ConnectionSettings` only when Backend sees PostgreSQL confirm it.
The Frontend's protections are: the in-batch fail-closed projection for simple Query, the `ForwardThenFlush` plus driver drain (`driver.rs` L214-L267) for extended Execute, and the Parse-to-Bind and Bind-to-Execute snapshot comparisons in `SettingsProtocol`.

### 4.9 Schema change (R02, R03, R10)

DDL intent is recorded per statement, portal and execution; Backend applies it to the overlay on success; on commit `Context::publish_schema_if_changed` drives `SchemaManager::reload`; other connections adopt the new snapshot at their next `prepare_schema_for_statement` or `prepare_parse`, only when idle and clean.
A stored `Statement` is not re-derived after adoption.

### 4.10 COPY (R01, R02, R10)

`COPY <table> FROM` reaches `check_copy_into` only when mapping is on, `unsafe_disable_mapping` is false and `has_encrypted_columns()` is true.
It passes only for a uniquely resolved table with a non-empty all-`Native` target list and no unquoted `like` column.
`COPY ... TO` and `COPY (query)` forms are not `copies_into_table`; they are outside `requires_type_check` and are forwarded as `NotTypeChecked` unless `writes_data` and `has_encrypted_columns` both hold.
Subsequent COPY data messages take the default arm of `intercept`.

## 5. Cross-function invariants

These hold only across functions; no single record states them.
Each lists what holds it up and what does not.

### 5.1 Positional alignment chain for literals

`typed_statement.literals[i]`, `literal_columns[i]`, `inbound[i]`, `skip[i]`, `plaintexts[i]`, `encrypted[i]` and the node-key map handed to `transform` all refer to the same literal.
Held by: shared iteration order of one `Vec` (R11, R12); `ColumnMapper::get_column` never returning `Ok(None)` (R02, R03, R11); `Context::encrypt` enforcing output length equals input length (R06, R11); `place_ciphertexts` checking count (R04); `check_all_encrypted_literals_provided` checking non-Null count and key membership (R12).
Not held by anything found: order of the encryption service's outputs (R02, R04, R06, R11); every zip on the chain truncates silently and none of `encrypt_literals`, `merge_inbound_eql`, `transform_statement` checks lengths itself (R07, R11, R12).

### 5.2 Positional alignment chain for parameters

`statement.output_params[i]`, `inbound[i]`, `skip[i]`, `plaintexts[i]`, `output_param_columns[i]`, `encrypted[i]` and the i-th outgoing Bind parameter correspond.
Held by: each being a map over `output_params` (R06); the length check in `Context::encrypt`.
`Bind::rewrite` does not validate `encrypted.len()` (R04).
The link from `output_params` to the client's `$n` rests on `TypeCheckedStatement.params` being dense and ordered (R03) and on `output_params_from_plan`, whose zip truncates and whose index conversion uses `saturating_sub(1)` (R03).

### 5.3 One routing decision per statement text, reused for its whole life

`route` runs at Query and at Parse only (R01).
For a prepared statement, the Map-or-Forward outcome is frozen in `prepared_statements[name].statement` and reused by every Bind (R03, R04), Describe (R05) and, through the portal, every Execute and result row.
What re-validates it later: only `SettingsProtocol::bind` and `::execute`, only for settings, only when `prepared[name]` or the portal entry exists and `needs_mapping` is true (R04, R05).
Nothing found re-validates it against a schema snapshot adopted after Parse (R03, R10).
`check_copy_into` shares this property: a prepared `COPY ... FROM` is checked at Parse only (R10).

### 5.4 The mapped-statement and mapping-disabled branches are kept apart by SettingsProtocol alone

`bind_handler` takes the passthrough branch whenever the connection's `unsafe_disable_mapping` is true, before looking at the stored `Statement` (R04).
`route` reads the same setting afresh at Parse (R01).
That a statement mapped at Parse cannot be bound on the passthrough branch depends on `SettingsProtocol::bind` refusing the mapping-flag mismatch, which requires `SettingsProtocol.prepared[name]` to exist for every name whose `PreparedStatements` entry holds a `Statement` (R04).
The two maps have separate removal paths; their agreement was not cross-verified (R04).

### 5.5 `has_encrypted_columns()` is a single gate shared by three write protections

`check_copy_into`, the nested-write refusal for non-type-checked statements, and the no-fallback-for-writes rule all sit behind `has_encrypted_columns() == true` (R01, R02, R03, R10).
It reads `encrypt_config` and the `base` snapshot, not the transaction overlay resolver (R10).
All three therefore share one freshness condition: the adopted snapshot and encrypt config at the moment `route` runs.

### 5.6 Schema consistency within one routing decision

`route` reads `has_encrypted_columns`, the resolver, and `encrypt_config` under three separate locks, written sequentially by `adopt_latest` (R01, R03).
Agreement across the three reads rests on: `CipherStashMiddleware` taking `&mut self` for both frontend and backend operations (R05); no await between the reads (R03); and, for simple Query, the wait for `in_flight_ddl == 0` (R02).
The wait and `ensure_schema_modelled` run only when some statement `requires_schema` (R02, R03).
A mismatch between resolver and encrypt config surfaces as `Err(UnknownColumn)` for an EQL-typed node (R01, R03).

### 5.7 Keyset coherence within one handler invocation

A single `encrypt_literals` or `encrypt_params` call reads the effective keyset up to three times across awaits: local encryption, inbound decrypt, verification re-encrypt (R06, R07, R11).
`prepare_parse` and `bind_settings_statement` separately snapshot `EffectiveSettings` (R03, R04).
Coherence rests on the per-connection serialisation in 5.6 and on no Backend transition running while a Frontend handler is suspended.
The inbound payload's embedded keyset id is checked against the decrypt cipher only (R07, R09).
No record found the keyset stored with the ciphertext result, the `Statement`, or the portal; what is stored is `literal_sensitive` on the prepared entry and `Bound.settings` on the settings portal (R03, R04, R06).

### 5.8 Keyset coherence between phases

Ciphertext is produced at Parse (literals) and at Bind (parameters) and consumed at Execute.
Literals: a keyset change between Parse and Bind refuses the Bind when `literal_sensitive` (R03, R04).
Parameters: a statement with encrypted params only is bound under the keyset current at Bind (R04); Bind-to-Execute drift is covered by `SettingsProtocol::execute` when the portal exists and `needs_mapping` (R05).
Simple Query: the in-batch projection plus the driver drain (R02).

### 5.9 Inbound Store acceptance is a five-part conjunction spread over four functions

A client Store payload reaches PostgreSQL only if: `parse` validated version, identifier, descriptor string and term presence against the column (R08); `decrypt_eql` found its keyset id equal to the cipher's and the cipher accepted every record (R07); `ZeroKms::encrypt` chose `EqlOperation::Store` for the destination's `eql_term` (R07); and `sem_terms_match` found version, identifier and SEM terms equal (R09).
Combining R08 with R07: a `Store(SteVec)` payload requires a column with a SteVec index, and on such a column the JSON operand terms map to a Query operation, so the derived-is-Store test leaves `Full`, `Partial` and `Tokenized` positions for SteVec stores; a `Store(Encrypted)` payload requires a column with no SteVec index.
This consequence is derived from the two records and was not traced in code; R08 lists it as open.
What the conjunction does not compare, taken across R07, R08 and R09: scalar record bytes (iv, ciphertext, tag, decryption_policy) beyond the black-box decrypt; SteVec `kind`; SteVec `key_header` beyond descriptor and keyset id; non-root SteVec entry ciphertexts beyond per-entry acceptance by the black-box decrypt; any row or single-use binding.

### 5.10 Inbound Query acceptance is structural at every stage

`parse` checks shape, role (`query_operand` true), `eql_term` and SteVec configuration (R08).
`merge_inbound_eql` writes it to the slot without reading the column (R07).
`project_query_operand` passes it through unchanged (R06, R11).
`transform_statement` serialises it into a string literal, and a `Selector` is embedded verbatim (R12).
No stage contacts ZeroKMS for a Query payload (R06, R07).
The remaining check, if any, is in the upstream EQL v3 query domains, which were not inspected (R07, R08).

### 5.11 Content-based classification is shared by literals and parameters

The same `inbound_eql::parse` decides plaintext versus payload for both SQL literals (text from `Value::into_string`) and Bind parameters (raw bytes, with one leading `0x01` stripped for binary format without consulting the type OID) (R06, R08, R11).
A storage-shaped or query-shaped object that fails validation is an error, never plaintext (R08).

### 5.12 Refusal means none of the client's SQL is forwarded, but not none of the client's data

Query and Parse refusals forward no client SQL (R05).
A Bind refusal forwards the client's parameter values unchanged under a random statement name (R04, R05).
A Bind that fails `Bind::try_from` is not a refusal at all; it is a connection-level error (R05).

### 5.13 Local state written before a refusal

Several writes precede the point at which a handler can still return `Err`, and the Frontend-side refuse functions do not undo them:

- `operations[op].deallocations` in `query_handler` (R02).
- `SchemaMiddleware.prepared[name]` in `prepare_parse` (R03).
- `SettingsProtocol` journal and removal of the previous portal, `SchemaMiddleware.portals[portal]`, and `preparation = Bind` in `begin_bind` (R04, R05).
- `begin_parse` has already removed the previous statement under the name (R03).

Whether each is later reverted depends on Backend journal replay and boundary handling, which were not read.

### 5.14 Proxy's discard state is two flags with different owners

PostgreSQL's ignore-till-Sync state is mirrored by `failed_extended_batch` (Frontend-owned, cleared only by Sync) OR `discarding_until_sync` (set by Backend, cleared by Sync and at readiness) (R05).
The gate applies before the operator mapping-disabled branch, but `failed_extended_batch` is never set on that branch.

### 5.15 Analysed text versus forwarded bytes

On every path where nothing is rewritten, Proxy decides on a lossy-UTF-8, sqltk-parsed view and forwards the client's original bytes (R02, R03, R10).
On the simple-Query rewrite path the opposite holds: every statement, including Forward routes, is re-rendered from the AST and no client text is reused (R02, R12).
On the extended rewrite path only a transformed statement is re-rendered (R03).
The three paths therefore depend on three different equivalences: sqltk parse equals PostgreSQL parse; AST Display round-trips; and `SingleQuotedString` rendering plus the upstream string-literal settings preserve embedded JSON.

### 5.16 One portal per simple Query

`register_query` installs one unnamed portal for the whole batch, taken from the last Map route (R02).
Backend reads one portal per operation.
`refuse_query` and `refuse_execute` also use the unnamed portal name (R05).
The relationship between that single portal and the row shapes of each statement in a multi-statement batch was not established by any record.

### 5.17 Fallback is a narrow, single-site decision with a default-open classification

`Forward::Fallback` arises at one site, only when `enable_mapping_errors` is off (the default), the statement is not an encrypted write, and the error is `StatementCouldNotBeTypeChecked` or `Internal` (R01).
`mapping_error` reaches those two variants through its default arm for any mapper error whose wrapper shape the detectors do not match, and for unresolved tables on reads (R01).
`query_handler` adds the batch rule that a fallback may not accompany a Map route (R02).
A fallback at Parse stores `statement: None`, so later Binds are passthrough and results are not decrypted (R03, R04).

## 6. Places where records disagree or differ in confidence

Both sides are cited; neither is preferred here.

1. Frontend/Backend serialisation.
   R05 states as an invariant that Frontend and Backend intercepts are serialised per connection because both take `&mut` on one `CipherStashMiddleware` (`middleware/mod.rs` L54-L76).
   R02 cites the comment at `driver.rs` L226-L228 that the intermediary is single-task.
   R01, R04, R06, R07, R09 and R11 each list as unresolved whether Backend can mutate settings or schema state while a Frontend handler is suspended at an await, and R03 says exclusivity across other `Context` clones is unverified.

2. `ColumnType::Timestamp` reachability.
   R04 states that Timestamp is produced at `proxy/encrypt_config/from_domain.rs` L79, so the `unimplemented!()` arms at `data/from_sql.rs` L381-L383 and L506 have a producer.
   R06 and R11 list as unresolved whether a column config can carry `ColumnType::Timestamp` at all.

3. Bloom filter comparison.
   R09 states the comparison is sorted-multiset equality without deduplication and notes the source comment calls it a set.
   R11 describes it as "compared as a set".
   R07 says "bf (sorted)".

4. Scope of identifier checking for Query payloads.
   R07's call notes describe `parse` as the gate for Query payloads with "shape/version/identifier/eql_term rules".
   R08 and R06 state version and identifier are checked on the `Encrypted` arm only; the `SteVec` and `Selector` arms compare no identifier.

5. Whether every Parse error is followed by a working `refuse_parse`.
   R03 states as an invariant that every `Err` exit of `parse_handler` is followed by `refuse_parse`, which removes the entry and stores the diagnostic.
   R05 notes `refuse_parse` returns `UnknownOperation` when no Parse preparation exists, which holds only for errors raised between `begin_parse` and `finish_parse`, and lists as unresolved whether `parse_handler` can return `Err` after `finish_parse` (`frontend.rs` L876-L887).

6. Line extent of `check_copy_into`.
   R01 gives L1400-L1455; R10 gives L1396-L1455.

7. Whether the keyset at Parse is tied to execution.
   R11 says enforcement through `capture_literals` was "not traced".
   R04 describes the mechanism: `SettingsProtocol::bind` refuses a keyset mismatch when `literal_sensitive` (`settings_protocol.rs` L213-L219).
   These are compatible but differ in how far the path was followed.

8. Metrics gating.
   R06 reports the Bind-path counters gated on `prometheus_enabled`; R11 reports the literal-path counters ungated.
   This is an asymmetry between two paths rather than a contradiction.

## 7. Where complexity concentrates

1. Three interleaved per-connection state machines (`ConnectionProtocolState`, `SettingsProtocol`, `SchemaMiddleware`) written optimistically by the Frontend and confirmed or rolled back by the Backend through journals and boundaries keyed by operation id.
2. The routing decision in `route`/`mapping_error`: an ordered chain of gates whose first branch (`unsafe_disable_mapping`) bypasses all others, and whose default arm classifies unrecognised mapper errors as coverage gaps.
3. Parse-time decisions with Bind-, Execute- and row-time consumers, with re-validation limited to settings.
4. The inbound EQL path: content-based detection, two validation regimes (authenticated Store, structural Query), and reliance on black-box cipherstash-client behaviour for determinism, deserialisation and authentication.
5. Positional pairing through repeated `zip` with length enforced at two remote points and order enforced nowhere.
6. Dual views of SQL: lossy-decoded sqltk AST for decisions, original bytes or AST Display for forwarding.
7. Schema freshness: three locks, conditional adoption, a no-timeout wait gated on `requires_schema`, resolver name matching that does not track `search_path`.
8. Local refusal by substitute messages that depend on PostgreSQL erroring, and discard state mirrored by two flags.
9. Multiple independent reads of the effective keyset inside one handler, across awaits, against a process-wide cipher cache keyed by a client-selectable string.
10. `Bind::rewrite` positional versus rebuild paths, client-declared OIDs driving decode, and panicking decode arms.
11. Simple-Query batches: per-statement resolvers, batch rules, one portal, completion-count alignment.
12. `check_copy_into` and identifier matching (`IdentCase` Unicode lowercasing versus ASCII folding of overlay keys, unquoted catalog names).

## 8. Unenforced assumptions (verbatim from the records)

Each of the following was recorded as established by nothing.
They are carried verbatim and are assumptions, not findings.

### Frontend::route

- Statement kinds outside requires_type_check (EXECUTE, CALL, DO, COPY ... TO, CREATE TABLE ... AS, cursors, DDL) carry no plaintext bound for an encrypted column and need no result decryption, so Forward::NotTypeChecked is sufficient.
- has_encrypted_columns(), encrypt_config() and the resolver argument describe the same schema generation at the time route runs.
- has_encrypted_columns() == false means no write in the statement can reach an encrypted column, so COPY FROM skips check_copy_into, nested writes are forwarded, and a failed write may fall back.
- A table name resolves to the same table in PostgreSQL as in the resolver (default search_path, pg_temp first, one-part name matches a table in any loaded schema).
- The route chosen at Parse, including the unsafe_disable_mapping value read at L1254, remains valid for every later Bind/Execute of the prepared statement.
- param_types are meaningful PostgreSQL type OIDs.

### Frontend::query_handler

- sqltk splits and classifies the SQL into the same statements, in the same order, that PostgreSQL executes from the forwarded bytes.
- The analysed text equals the forwarded bytes.
- AST Display re-rendering preserves the meaning of statements routed Forward when the batch is rewritten.
- A statement for which requires_schema is false needs no protection from a preceding settings change or encryption-changing DDL in the same batch.
- ddl_may_change_encryption returns the same answer at the fail-closed check (frontend.rs L431, before the wait) and inside batch_resolvers (middleware.rs L880, after the wait).
- The single registered portal describes the result rows of every statement in a multi-statement Query.
- PostgreSQL emits one completion or error per forwarded statement, in order, keeping the schema execution queue and settings command queue aligned.
- Schema intent and settings command of the transformed ASTs passed to register_query equal those of the parsed ASTs checked at L412-L434.
- EncryptionService::encrypt returns outputs in input order bound to the given columns and keyset.

### Frontend::parse_handler

- The text analysed equals the bytes PostgreSQL executes on the no-rewrite path; analysis uses a lossy UTF-8 decode (L815) while original bytes are forwarded (L890-900)
- sqltk's parse of the SQL matches PostgreSQL's parse of the same bytes on Forward and on Map-without-transform
- The Parse holds exactly one statement; the AST covers all of the forwarded text
- The route decided at Parse (Some(Statement) or None) stays valid against the schema for the lifetime of the statement name
- Resolver used for type checking and encrypt config used for column lookup describe the same schema
- Number of client-declared parameter types matches the statement's placeholders
- Client OIDs fit in i32; the cast value is stored and used at Bind to select a decode type
- Proxy's statement-name key (exact client bytes) identifies the same object as PostgreSQL's; deallocation names are ASCII-lowercased with no length limit
- Schema-middleware prepared[name] intent written during prepare_parse is discarded when the Parse is later refused

### Frontend::bind_handler

- The settings snapshot stored in Bound.settings (context/mod.rs L1466), the mapping flag (lifecycle.rs L239) and the keyset read by each encrypt/decrypt call (context/mod.rs L1658, L1689) are the same settings, although the latter reads occur after .await points.
- statement.output_params is empty whenever statement.param_columns is empty, so the has_params gate does not skip outputs needing encryption.
- The number of bound values matches what the statement expects.
- result_columns_format_codes count is 0, 1, or equal to the projection width.
- Parameter decoding returns Err rather than panicking for every column type.
- Schema-middleware portal intent written at begin_bind is reverted if the Bind is later refused locally.
- Text-format parameter bytes are valid UTF-8.

### Frontend::intercept

- PostgreSQL answers every substitute message with an ErrorResponse (DO/RAISE EXCEPTION block, cast to type __cipherstash_<uuid>, Bind of statement __cipherstash_<uuid>, Execute of portal __cipherstash_<uuid>). If the substitute completes, finish_execution(Completed) drops the stored diagnostic (context/mod.rs L363-L372).
- quote_literal produces exactly one SQL string literal for every diagnostic field value, including values that embed client-supplied names and EQL Mapper error text, when applied twice (option value, then DO body).
- The upstream role may run an anonymous DO block in the default procedural language.
- Every portal PostgreSQL can execute by name was created by a Bind that Proxy processed, so an Execute with no Proxy portal entry has nothing to map or decrypt.
- FunctionCall bodies and all variants reaching the `_` arm (including COPY data messages and Flush) carry nothing that requires mapping.
- PostgreSQL's discard-until-Sync state equals failed_extended_batch OR discarding_until_sync at the moment each frontend message is intercepted.
- PostgreSQL processes Terminate while discarding a failed extended batch.
- An ErrorResponse for a substituted Parse or Bind reaches Proxy without a requested flush (those arms use Forward), whereas the refused Execute requests ForwardThenFlush.

### Frontend::encrypt_params

- Every path-step param index of a fused JSON selector is bound
- The client binds exactly as many params as the statement declares
- A binary-format param whose first byte is 0x01 is jsonb with a version header
- The effective keyset is identical across the three reads in one call (encrypt, inbound decrypt, verification re-encrypt)
- The keyset in effect at Bind is the keyset the value must carry when the portal executes
- A column typed with a JSON query term (JsonPath, JsonAccessor, JsonOrd, JsonValueSelector) has a SteVec SEM term configured
- Inbound SteVec and Selector query payloads belong to the destination column

### Frontend::merge_inbound_eql

- SEM term values in a Query payload (hm, bf, ob, op, selectors, SteVec terms) were derived under the connection's effective keyset.
- A SteVec-shaped Query payload belongs to the destination column.
- The effective keyset is identical for the decrypt at L1187 and the re-encrypt at L1209.
- Re-encryption is deterministic for hm, ob, op, SteVec selectors, SteVec terms and SteVec entry order, so exact equality is a valid test.
- Authenticating each non-root SteVec entry under its selector ties that entry's ciphertext content to the root plaintext.
- The decrypted Plaintext variant suits the destination column's cast_type.
- An accepted Store payload is bound to its row or to a single use.
- The count of Store payloads per statement and SteVec entries per payload is bounded before decrypt records are built.

### inbound_eql::parse

- A Query payload's term content (hm, bf, ob, op, SteVec entries) is acceptable without authentication, length, or format checks.
- A Selector token belongs to the destination column and effective keyset.
- A Query(SteVec) payload belongs to the destination column.
- In a selector query position a 32-char lowercase-hex value is an application-generated token and not a plaintext selector.
- A Store payload is appropriate for the position's eql_term.
- Input size is bounded before parse builds a full serde_json::Value tree.

### sem_terms_match

- encrypt_eql_v3 reproduces identical hmac, ORE, OPE, bloom multiset, and SteVec entry count/order/selectors/array markers/terms for the same plaintext, column config and keyset.
- The keyset used to derive equals the keyset that authenticated the inbound ciphertext.
- Uncompared SteVec `kind` holds an acceptable value.
- Each non-root SteVec entry's ciphertext encrypts the value that belongs at its selector in the root plaintext.
- Serialization of SteVecEntryTermV3 is injective and Some(term) never serializes to null.
- PartialEq on Identifier and on the hmac/ORE/OPE/selector field types is full structural equality.
- Client JSON keys outside the struct definitions do not survive into the stored value.

### check_copy_into

- The table the model resolves for a bare name is the relation PostgreSQL resolves for the client session; the model matches any loaded schema and the loaded schemas are current_schemas(false) of Proxy's own loader connection, with no tracking of the client's search_path
- IdentCase comparison matches PostgreSQL identifier resolution; catalog table names are stored unquoted (manager.rs L411) and unquoted comparison uses Unicode lowercasing while overlay keys fold ASCII only
- A prepared COPY checked at Parse is still valid when later executed
- Copy data lands only in the listed columns of the named table (no database-side redirection through triggers, rules, partition routing, generated columns or defaults)
- table_name and columns from the sqltk AST name the same table and columns PostgreSQL parses from the unchanged forwarded text
- The resolver RwLock is not poisoned (read().unwrap())

### Frontend::encrypt_literals

- Value::into_string returns Some for exactly the literal forms whose text can carry an inbound payload
- The keyset is the same for encryption, inbound authentication, and SEM re-derivation; each is a separate read of protocol_state across await points
- A client-supplied query-only payload carries SEM term values acceptable for the column
- A 32-character lowercase-hex literal in a JsonAccessor/JsonPath position is an already-tokenised selector, not a plaintext field name
- A JSON object literal with v, i and one of c/h/sv is an EQL storage payload, not plaintext data
- encrypt_eql_v3 returns outputs in input order, in the shape implied by EqlOperation, and rejects a Plaintext variant that mismatches cast_type (from_sql.rs L68 and L76-L78 produce Boolean/Text regardless of column type)
- EqlCiphertext::into_query_operand removes the decryptable ciphertext and keeps SEM terms
- ColumnType::Timestamp with a Full/Partial term never reaches text_from_sql, whose arm is unimplemented!() and panics
- JSON term variants are only assigned to literals on columns with a SteVec index; otherwise ZeroKms::encrypt falls back to EqlOperation::Store
- session_id names a live metrics scope

### Frontend::transform_statement

- encrypted_literals[i] corresponds to typed_statement.literals[i]; the zip at L743-L746 pairs by position and truncates silently.
- Rendering Value::SingleQuotedString yields a SQL literal whose value equals the Rust string for any content (quotes, backslashes, pre-doubled quotes).
- The upstream session reads backslashes inside '...' literally, so serde_json escape sequences in the embedded JSON survive.
- A Selector string is a 32-character lowercase hex token.
- Cast target names taken from the schema model render as valid identifiers without quoting.

### Recurring themes among these

Grouping only; the wording above is authoritative.

- Parser equivalence between sqltk and PostgreSQL, and text-versus-bytes equivalence (route, query_handler, parse_handler, check_copy_into, transform_statement).
- Longevity of Parse-time decisions (route, parse_handler, check_copy_into).
- Schema-generation and name-resolution agreement (route, parse_handler, query_handler, check_copy_into).
- Keyset identity across reads and across phases (bind_handler, encrypt_params, merge_inbound_eql, sem_terms_match, encrypt_literals).
- Unverified content of client Query payloads and selectors (encrypt_params, merge_inbound_eql, inbound_eql::parse, encrypt_literals, transform_statement).
- Black-box cipherstash-client properties: order, determinism, type checking, equality, serialisation (query_handler, merge_inbound_eql, sem_terms_match, encrypt_literals).
- Upstream PostgreSQL behaviour for substitute messages and discard state (intercept).
- Counts and bounds supplied by the client: parameter types, bound values, format codes, payload size (parse_handler, bind_handler, encrypt_params, merge_inbound_eql, inbound_eql::parse).
- Rollback of state written before a refusal (parse_handler, bind_handler).

## 9. Assumptions established elsewhere (pointers)

These were recorded with an establishing site in another function; they are listed so a reader can check the link.

- The multi-statement batch rules are applied by `query_handler` (`frontend.rs` L456-L474), not inside `route` (R01).
- The operator switch and settings/DDL fail-closed checks run before `route` (`frontend.rs` L189-L211, L412-L438; `lifecycle.rs` L148-L158) (R01).
- A returned `InboundEql::Store` is authenticated and compared in `merge_inbound_eql` (`frontend.rs` L1185-L1238); each caller of `parse` must route to it (R08).
- The keyset named in a Store payload is checked in `ZeroKms::decrypt_eql` (`zerokms.rs` L401-L408), `AllEntries` path only (R08).
- `encrypted.len()` covers every index used in `merge_inbound_eql`, by the callers' construction and `Context::encrypt` (R07).
- `query_operand` is supplied by eql-mapper (`type_checked_statement.rs` L238-L248, `query_operands.rs` L49-L56) and is constant `false` for untransformed statements (`frontend.rs` L945) (R06, R08).
- Mapped statements stay off the mapping-disabled Bind branch through `SettingsProtocol::bind` (R04).
- Earlier settings-changing executions are drained before the next frontend message by `driver.rs` L214-L267, which has an early exit at L241-L245 (R02, R04).
- The descriptor compared at parse time is bound to the ciphertext by the AEAD tag, per a comment at `inbound_eql.rs` L104-L106 only (R07, R08).
- `decryption_policy` is verified during key retrieval, per a comment at `zerokms.rs` L410-L412 scoped to tag-version 1 (R07).

## 10. Unresolved questions (carried forward in full)

Merged where several records ask the same thing; the tags say which.

### 10.1 pg-proto, driver and Backend

1. pg-proto 0.12.0 source is not on disk: exact semantics of `Suppress`, `ForwardThenFlush`, operation-id ordering, and how a middleware `Err` is surfaced on each forwarding path (R05).
2. Whether the middleware factory is called once per connection, whether frontend and backend operations are serialised, and whether a `frontend_operation` future can be dropped mid-await between `begin_parse` and `finish_parse` (R03).
3. Whether Backend can mutate `SchemaMiddleware` (`adopt_latest`, `apply_ddl`, temporary tables) or `protocol_state.settings` on another task while a Frontend handler is suspended, in particular at `frontend.rs` L436, L669, L1046, L1118, L1188, L1210; which task runs `Context::settings_execution_succeeded` (`lifecycle.rs` L454) and `ConnectionSettings::command_succeeded` (R01, R04, R06, R07, R09, R11).
4. Whether `in_flight_ddl` can be non-zero on entry to `query_handler` or `prepare_parse`; what advances the backend leg while `frontend.rs` L436 or `lifecycle.rs` L150 awaits with the middleware exclusively borrowed; the early break at `driver.rs` L241-L245 (R02, R05).
5. How `ForwardThenFlush` maps to `ForwardedMessage`/`FrontendExpanded` in the driver, and whether settings-changing executions are always drained before the next frontend message reaches `intercept` (R02, R04).
6. What the client receives for a non-Sync message suppressed at `frontend.rs` L185-L186, in particular a simple Query sent while `failed_extended_batch` is true and no Sync follows (R05).
7. Whether `failed_extended_batch` can remain true after PostgreSQL has left its discard state, given ReadyForQuery does not clear it (R05).
8. Whether PostgreSQL processes Terminate during ignore-till-Sync as the comment at `frontend.rs` L174-L175 states (R05).
9. Whether any path lets PostgreSQL hold a named portal or cursor that Proxy has no entry for, given the Execute arm has no existence check (R05).
10. Whether `parse_handler` can return `Err` after `finish_parse` consumed the preparation (`frontend.rs` L876-L887), turning `refuse_parse` into `UnknownOperation` (R05).
11. Why the refused Execute sets `flush_after_forward` while refused Parse and Bind do not, and how the driver's outcome drain treats each (R05).
12. Backend handling when a refused operation's substitute message is itself discarded by PostgreSQL after an earlier pipelined failure, leaving `operations[op].error_response` with no ErrorResponse to replace (R05).
13. Backend handling of ParseComplete and ErrorResponse: whether `preparation_acknowledged`/`postgres_failed` run with the Parse's operation id, including for the substitute Parse (R03).
14. How the schema execution queue and settings `Pending` behave when PostgreSQL's completion count differs from `forwarded_statements.len()`; `Backend::intercept_backend` past `backend.rs` L180 and `SettingsProtocol::ready`/`failed` (R02).
15. How Backend handles rows of a multi-statement Query with differing projections given one portal per operation; `backend.rs` L473-L560, `data_row::as_ciphertext`, `context/mod.rs` L1242-L1270 (R02).
16. How `Backend::data_row_handler` and `Portal::format_codes` call sites handle a client result-format-code count that differs from the row width (R04).
17. Whether the Query string carries the protocol trailing NUL at `frontend.rs` L410; NUL handling in `pg_proto::Parse` query and statement fields and bounds on `parameter_types` (R02, R03).
18. Whether any size limit on Bind params or SQL literals applies before `inbound_eql::parse`; pg-proto framing limits and `Bind::try_from` (R08).
19. How CopyData after an accepted `COPY FROM STDIN` is handled by `intercept` and the pg-proto pipeline (R10).
20. The connection task's panic handling in `main.rs` for the `unimplemented!()` arms at `data/from_sql.rs` L382 and L506 (R04).
21. State of `SchemaMiddleware.portals[portal]` after a locally refused Bind; rollback of `SchemaMiddleware.prepared[name]` after a refused or rejected Parse through `execution_failed`, `protocol_boundary`, `discard_skipped_executions`, `restore_failed_preparation` (R03, R04).

### 10.2 SQL parsing and rendering (sqltk)

22. Whether the sqltk visitor behind `writes_data` reaches a write inside `CopySource::Query`, `CREATE TABLE ... AS`, `CREATE VIEW`, `DECLARE CURSOR` or a function body (R01).
23. Behaviour of sqltk `Parser::parse_statement` on tokens after the first statement and on an empty query string (R03).
24. Whether AST Display round-trips every statement kind sqltk accepts (R02).
25. The sqltk `CreateTable` AST for `PARTITION OF`, typed tables (`OF type`) and mixed column plus `LIKE` lists, which `is_modelled_ddl` (`middleware.rs` L1171-L1180) does not test (R10).
26. Whether sqltk `parse_copy` accepts target forms PostgreSQL reads differently from the AST (R10).
27. Which `Value` variants `into_string` returns `Some` for, how it renders escaped literals, and which literal kinds return `None` (R08, R11).
28. How `Value::SingleQuotedString` is rendered when content contains `'`, `\`, `''` or `\'` (R12).
29. Whether `standard_conforming_strings` can be off on the upstream session; connection startup parameters and SET forwarding (R12).
30. `decimal_from_sql` handling of `Value::Number` per `ColumnType` (R11).

### 10.3 eql-mapper

31. Which type-check failures on reads of encrypted columns reach the default arm of `mapping_error` (`frontend.rs` L1383); `TypeError`, `ScopeError`, `ImportError` variants and raise sites (R01).
32. Whether `UnmappableEncryptedColumn` can be wrapped as `Scope(ScopeError::TypeError)` or `Import(ImportError::ScopeError)`, which `as_unmappable_encrypted_column` does not match (R01).
33. Whether `requires_transform` is true for every statement with an encrypted literal; what it returns when the dry run fails (`type_checked_statement.rs` L152); whether a statement with an encrypted predicate param can keep default `output_params` with `query_operand` false (R01, R03, R06).
34. Ordering and density of `TypeCheckedStatement.params` from `registry.resolved_param_types` (R03).
35. Whether `output_params` is empty whenever `param_columns` is empty, per `ParamPlan` construction (gate at `frontend.rs` L1045) (R04).
36. Whether a transformed statement can classify differently for `SettingsCommand::from_statement` or `SchemaMiddleware::intent` (R02).
37. Whether `SubstituteEncryptedLiterals::check_postcondition` runs on the real transform; any hidden call in sqltk `apply_transform` or the `impl_for_tuples` expansion (R11, R12).
38. `registry.get_nodes_and_types::<ast::Value>()` and AST positions where `Value` occurs outside `Expr`; whether every EQL-typed literal sits under `Expr::Value` (R12).
39. Whether `typed_statement.literals` can list the same node twice (R12).
40. `RenumberParams`, `FailOnPlaceholderChange`, `ParamPlan::check_covers` and the remaining rewrite rules were not read line by line (R12).
41. Whether a JSON term variant can be assigned to a literal or param on a column without a SteVec index, making the Store fallbacks at `zerokms.rs` L277-L316 reachable; eql-mapper trait bounds and `proxy/schema/eql_domains.rs` (R06, R11).
42. What the upstream statement's declared param count is relative to a Bind rebuilt by `Bind::rewrite` when the client bound surplus params; `rewrite_parse_param_types` (R06).

### 10.4 Schema model

43. Whether a column made encrypted inside the current transaction is always reflected in `encrypt_config`, given `schema_has_encrypted_columns` reads `base` (`middleware.rs` L383); `apply_ddl` and `apply_encrypt_config` (R01).
44. Whether every Eql-classified DDL column also yields encrypt config, since `has_encrypted_columns` does not read the overlay resolver; `eql_domains.rs` and `column_config_from_domain` (R10).
45. Whether the LIKE-as-column parse (`frontend.rs` L1436-L1439) is the only way a loaded table carries a placeholder column; `collect_ddl_with_column_kind` (R01).
46. Which statements outside the match at `middleware.rs` L556-L609 change what a later `requires_schema` statement resolves (`SET search_path`, `ALTER TABLE SET SCHEMA`, `CREATE SCHEMA`, `DO`, `CALL`); `middleware.rs` L1123, L1137, L1169 (R02).
47. Whether `bind_handler`, `Context::begin_bind`, `SchemaMiddleware::bind`/`execute` or any execute handler re-validate a stored route, or a prepared `COPY ... FROM` with `finish_parse(operation, None)`, against a schema that changed after Parse (R03, R10).
48. `information_schema.tables` semantics for partitioned parents, partitions and foreign tables under `table_type = 'BASE TABLE'` (`select_table_schemas.sql` L20) (R10).
49. Whether the loader's `current_schemas(false)` can differ from a client session's search path; `connect::database` and `DatabaseConfig` options (R10).
50. Whether the temporary layer can hold non-Native columns; `SchemaMiddleware::execution_succeeded` (R10).
51. What characters a domain name can carry into `eql-mapper/src/transformation_rules/helpers.rs` L145-L146; `proxy/schema/manager.rs` and `domain_identity()` (R12).

### 10.5 Settings and naming

52. What `bind_handler` does when the `unsafe_disable_mapping` value at Bind differs from the one `route` saw at Parse; `bind_handler` around `frontend.rs` L1011 and `BindKnowledge` (R01).
53. Whether a state exists where a `Statement` is stored but `SettingsProtocol.prepared[name]` is absent; `SettingsProtocol` removal paths (`succeeded` L299-L300, `failed` L407-L415, `close`, `deallocated`) against `PreparedStatements` removal paths (R04).
54. Whether projecting with the empty `success_tag` (`settings.rs` L211-L217, `settings_protocol.rs` L57) matches confirmed-path behaviour for `COMMIT` and `SET LOCAL`; `settings.rs` L317 onward (R02).
55. How `capture_literals` handles a keyset change between Parse and Execute for statements with baked-in encrypted literals (R11).
56. PostgreSQL's treatment of protocol-level statement names longer than 63 bytes against Proxy's exact-bytes key (R03).

### 10.6 Inbound EQL and cipherstash-client

57. Whether `inbound_eql::parse` role checks cover every literal position; what is accepted when a SQL literal is itself an EQL payload; what `merge_inbound_eql` yields for a Store payload at a position whose `eql_term` is `JsonAccessor`, `JsonPath`, `JsonOrd`, `JsonValueSelector` or `Tokenized` (R02, R03, R08).
58. Deserialisation of `EqlCiphertextV3`, `EncryptedPayloadV3`, `SteVecPayloadV3`, `SteVecEntryV3`, `EqlQueryPayloadV3`: variant selection; unknown-key handling and keys belonging to the other variant; objects with `v`, `i`, `sv` but no `h`, or with both `c` and `sv`, or `h` without `sv`; whether a JSON object can deserialise to `Selector`; which struct field the wire key `h` maps to; format or length checks on `hm`, `bf`, `ob`, `op`, SteVec entry `s` and `term`; variants of `EqlCiphertextV3` beyond `Encrypted` and `SteVec` (R07, R08, R09).
59. Whether `Identifier` has fields beyond table and column and how its `PartialEq` treats case and schema; `PartialEq` of the `hmac_256`, `ore_block_u64_8_256`, `ope_cllw` and selector field types (R08, R09).
60. Which values `SteVecKind` admits, how `k` deserialises, and what consumes `kind`, since no Proxy code reads it (R07, R09).
61. Whether `ob`, `op`, SteVec terms and SteVec entry order are deterministic across independent encryptions, and whether the root entry is always index 0 of the derived payload (R07, R09).
62. Whether `encrypt_eql_v3` rejects a `Plaintext` whose variant differs from `ColumnConfig.cast_type`; `PreparedPlaintext` (R07, R11).
63. Whether `encrypt_eql_v3` and `EncryptionService::encrypt` preserve input order, relied on by `place_ciphertexts` (`zerokms.rs` L510-L512) (R02, R04, R06, R11).
64. What `KeyHeader::record_with_selector` binds into nonce and AAD, the `Decryptable` implementation for `RecordWithNonce`, and what acceptance of a non-root entry by `ScopedCipher::decrypt` binds (selector, key_header, entry plaintext) (R07, R09).
65. Whether `decryption_policy` is authenticated for record tag versions other than 1 (R07).
66. What `EqlCiphertextV3::into_query_operand` removes and retains for `Encrypted` and `SteVec` payloads, including from a verified inbound payload (R06, R07, R09, R11).
67. Whether the `Serialize` impl of `SteVecEntryTermV3` is injective and whether any variant serialises to null (R09).
68. Which fields of `EqlCiphertextV3` and `EqlQueryPayloadV3` are free-form strings and whether unknown fields survive deserialisation; the JSON emitted per `EqlOutputV3` variant (R12).
69. Whether a `Selector` string returned by `encrypt_eql_v3` with `QueryOp::SteVecSelector` is always `[0-9a-f]{32}` (R12).
70. What checks the PostgreSQL `eql_v3.query_*` domains and other EQL v3 CHECK constraints apply to a client-supplied Query payload or selector token; the EQL SQL installed by the migrate command (R07, R08).
71. How serde_json 1.0.151 resolves duplicate top-level keys and which recursion limit and feature flags are in effect (R08).
72. Whether a column encrypt config can carry `ColumnType::Timestamp` and reach the `unimplemented!()` arms at `data/from_sql.rs` L381-L383 and L506; `proxy/encrypt_config/from_domain.rs` (R06, R11; see section 6 item 2 for R04's statement).

### 10.7 Diagnostics, logging and metrics

73. What `PostgreSqlErrorHandler::error_to_response` (`error_handler.rs` L37) exposes: the fallback error returned at `frontend.rs` L472; client SQL or schema names echoed by a Parse refusal; which fields of `InvalidParameter(Box<Column>)` and `UnknownPreparedStatement{name}` reach the client diagnostic (R02, R03, R04).
74. Which client-controlled strings reach `quote_literal` in `simple_error_query` (`frontend.rs` L1720-L1728); the diagnostics module and all `Error` Display impls (R05).
75. What the `?err` Debug output logged at `frontend.rs` L1194 and L1216 contains; Debug impls of cipherstash-client error types (R07).
76. Whether MAPPER/PROTOCOL debug targets are enabled in any shipped configuration, given the plaintext log at `frontend.rs` L1111 and the full Bind log at L1038; the log module and `LogConfig` defaults (R06).
77. Whether `StatementMetadata::set_encrypted_values_count` overwrites or accumulates across statements of one simple-query batch sharing a session id (R11).

### 10.8 Process and scope

78. The format resources `ANALYSIS_FORMAT.md`, `FUNCTION_MICRO_ANALYSIS_EXAMPLE.md` and `DOMAIN_NOTES.md` were not found (`{baseDir}` unresolved); confirm the prose layout is acceptable (R01, R02, R03, R06, R11).
79. Where the workflow expects the relative path `audit-context/functions/` to resolve; files were written under the repository root, not under `packages/cipherstash-proxy` (R06).
80. No per-function record exists for Backend, driver, Context lifecycle internals, settings, schema manager and middleware, the result-row decrypt path, `data_row.rs`, TLS, config, connect, migrate or prometheus; every statement in this dossier about those areas is second-hand (this synthesis).
81. Whether the derived consequence in 5.9 (SteVec Store payloads accepted only at `Full`/`Partial`/`Tokenized` positions on a SteVec column) holds in code; it combines R07 and R08 and was not traced (this synthesis; overlaps item 57).
