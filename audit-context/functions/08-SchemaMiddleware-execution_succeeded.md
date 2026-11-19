# SchemaMiddleware::execution_succeeded

- File: `packages/cipherstash-proxy/src/proxy/schema/middleware.rs`
- Lines: L704-L815
- Signature: `pub fn execution_succeeded(&self)`
- Kind: synchronous method, no arguments, no return value, no `await`, no I/O.
  It mutates per-connection state held behind `Arc<RwLock<_>>` and atomics on `SchemaMiddleware` (sm L313-L330).
- Format note: `ANALYSIS_FORMAT.md` was not found on disk (the `{baseDir}` placeholder did not resolve), so this file uses the section order of `audit-context/functions/06-SettingsProtocol-succeeded.md`.

Abbreviations used for citations:

- `sm` = `packages/cipherstash-proxy/src/proxy/schema/middleware.rs`
- `mgr` = `packages/cipherstash-proxy/src/proxy/schema/manager.rs`
- `ec` = `packages/cipherstash-proxy/src/proxy/encrypt_config/config.rs`
- `ed` = `packages/cipherstash-proxy/src/proxy/schema/eql_domains.rs`
- `ctx` = `packages/cipherstash-proxy/src/postgresql/context/mod.rs`
- `lc` = `packages/cipherstash-proxy/src/postgresql/context/lifecycle.rs`
- `be` = `packages/cipherstash-proxy/src/postgresql/middleware/backend.rs`
- `fe` = `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`
- `drv` = `packages/cipherstash-proxy/src/postgresql/driver.rs`
- `sd` = `packages/eql-mapper/src/model/schema_delta.rs`
- `tr` = `packages/eql-mapper/src/model/table_resolver.rs`
- `sc` = `packages/eql-mapper/src/model/schema.rs`

## 1. Purpose

`execution_succeeded` pops the entry at the front of the per-connection `executions` queue when that entry is an `Execute` (sm L705, L950-L960) and applies the schema meaning recorded in it.
The meaning is an `Intent` built on the Frontend path from the client's parsed SQL (sm L462-L470), not from anything PostgreSQL returned.
The function receives no command tag, no operation id and no statement identity (sm L704); selection is purely positional.
Depending on the popped statement it pushes a savepoint checkpoint, releases savepoints, restores a savepoint checkpoint, resets the overlay to the pinned base, or applies DDL to the catalog overlay, the encrypt config and the temporary-table layer (sm L707-L805).
For entries whose intent `changes_schema()` it recomputes `dirty`, decrements `in_flight_ddl` and wakes `wait_for_ddl` waiters (sm L806-L813).

## 2. Reachability and caller contract

### 2.1 Call chain

One production call site: `Context::report_schema_execution_succeeded` (ctx L1394-L1396), called only from `Context::execution_completed` (lc L456).
Test call sites: `Context::test_report_schema_execution_succeeded` (ctx L1959-L1960) and direct calls in the module's tests (sm L1269, L1285).

The production chain is:

1. `Backend::intercept` -> `Backend::intercept_backend` (be L148-L160).
2. `Backend::report_outcome` (be L89-L112), called from the passthrough branch (be L188), the discard branch (be L219), the failed-flush branch only when `operation.is_some()` (be L244-L246), and the normal path (be L254).
3. `report_outcome` calls `Context::execution_completed` for `CommandComplete` (be L96-L100), `EmptyQueryResponse` (be L101-L104) and `PortalSuspended` (be L105-L108).
4. `execution_completed` calls `settings_execution_succeeded(operation, tag)?` for `ExecutionEnd::Command` (lc L453-L455) and then `report_schema_execution_succeeded()` (lc L456).

### 2.2 What the caller does and does not gate on

- Command tag: available at lc L453, passed to the settings path (lc L454), not passed to the schema path (lc L456, sm L704).
- `operation`: `report_schema_execution_succeeded` runs before `if let Some(operation)` (lc L456-L457), so an uncorrelated terminal message pops a queue entry on the normal, passthrough and discard paths.
  The failed-flush path is the exception: it reports only when `operation.is_some()` (be L244-L246).
- `discarded`: not consulted before the schema call (lc L456-L459), so a completion the Backend suppresses from the client (be L218-L228) still applies its intent.
- `is_passthrough`: the passthrough branch reports outcomes (be L187-L188).
- `ExecutionEnd::Suspended` and `ExecutionEnd::Empty` reach the function exactly as `Command` does (lc L456); a `PortalSuspended` pops and applies an entry as a success.
- Settings failure: when `settings_execution_succeeded` returns `Err`, the `?` at lc L454 returns before lc L456 and the schema entry for that completion is not popped.
  `settings_execution_succeeded` returns `Err` only when the `protocol_state` lock is poisoned (ctx L1576-L1579).
  The error propagates through `report_outcome` (be L188, L219, L245, L254) and out of the driver loop (drv L253-L254).

### 2.3 Writers of the queue this function consumes

`executions: Arc<RwLock<VecDeque<PendingExecution>>>` (sm L321) holds `Execute(Option<Box<Intent>>)` and `ReadyBoundary` entries (sm L259-L264).

Pushes:

- `SchemaMiddleware::execute` (sm L511-L523): one `Execute(intent)` per extended-protocol Execute, where `intent` is `portals[portal].cloned()` and is `None` when the portal has no recorded intent (sm L512, L521).
  Reached from `Context::execute` -> `execute_schema_portal` (lc L312, ctx L1366-L1368), only when `config.mapping_disabled()` is false (lc L307-L310) and only after `execute_settings_portal` returned `Ok` (lc L311).
- `SchemaMiddleware::simple_query` (sm L526-L537): one `Execute(Some(intent))` per element of `statements`, then one `ReadyBoundary`.
  Reached from `Context::register_query` (lc L379) with `forwarded_statements` (fe L568-L569), which receives one push per sqltk-parsed statement (fe L480-L541: L518 or L525, L538).
- `SchemaMiddleware::protocol_boundary` (sm L667-L672): one `ReadyBoundary`.
  Reached from `Context::sync` (lc L366) and `Context::refuse_query` (lc L395).

Removals:

- `pop_execution` (sm L950-L960), called from `execution_succeeded` (sm L705) and `execution_failed` (sm L941).
- `discard_skipped_executions` (sm L963-L982), called from `ready_for_query` (sm L450): pops every entry up to and including the first `ReadyBoundary`, or to the end of the queue when no boundary is present (sm L966-L974).

Frontend paths that send a request PostgreSQL answers with a terminal or readiness message but push nothing to this queue:

- Operator-level mapping disabled: `unmapped_query` (lc L58-L63), `unmapped_sync` (lc L68-L70) and the early return in `Context::execute` (lc L307-L310) make no `schema_middleware` call.
  `mapping_disabled()` reads `self.config` (ctx L1794-L1796), so the queue stays empty for such a connection and `pop_execution` returns `None` (sm L957-L958).
- `FunctionCall` with mapping enabled: `Context::function_call` calls only `readiness_boundary` (fe L333-L335, lc L75-L77), which touches `settings_protocol` and `operations` (lc L81-L89) and pushes no schema `ReadyBoundary`.
  The comment at lc L72-L74 states PostgreSQL ends a FunctionCall with `ReadyForQuery`.
  Every `ReadyForQuery` reaches `schema_ready_for_query` through `became_ready` (be L200, L222, L359; lc L509-L517) and so runs `discard_skipped_executions` (sm L450).
- Refused Execute: when `Context::execute` returns `Err` the Frontend forwards an Execute for a fresh refusal portal name (fe L239-L254, L1756-L1760).
  An `Err` from `execute_settings_portal` (lc L311) precedes the schema push; an `Err` from `set_execute_for_portal` (lc L313) follows it.
  So the refusal Execute forwarded to PostgreSQL has a queue entry on one error path and none on the other.

### 2.4 Where the popped `Intent` comes from

- `prepare(name, statement)` stores `intent(statement)` under the prepared-statement name (sm L485-L488), called from `Context::prepare_parse` at lc L153.
  lc L153 runs before fallible steps in the same Parse: `validate_setting` (lc L158), `prepare_settings_statement` (lc L159) and `Frontend::route` (fe L834).
  When one of those fails the Parse is refused (fe L263-L273) and `prepared[name]` already holds the new intent.
- `bind(portal, prepared_statement)` copies `prepared[prepared_statement]` into `portals[portal]`, or removes `portals[portal]` when no intent is recorded (sm L491-L507), called from `Context::begin_bind` at lc L238 after `bind_settings_statement` succeeded (lc L237).
- No path removes entries from `prepared` or `portals` on Close or deallocation: `Context::close` makes no `schema_middleware` call (lc L317-L363), and the module documentation states intents are retained for the connection lifetime (sm L55-L57).
- `intent()` computes `ddl`, `modelled` and `temporary` at Parse or Query time against the encrypt config and base schema current at that moment (sm L462-L470, L474-L482, L618-L620, L655-L662).
  `Intent.modelled` is written at sm L466 and read nowhere in the file.

## 3. Inputs

The function has no parameters.
Its effective inputs are:

- The front entry of `executions` (sm L951-L953).
- `resolver`, `encrypt_config`, `base`, `base_encrypt_config`, `savepoints`, `temporary`, `unmodelled`, `local_unmodelled` as they stand at call time.

Origin of each field of the popped `Intent` (sm L85-L91):

- `statement`: the sqltk AST of client SQL after `mark_temporary` (sm L463, L235-L242); for a simple query it is the possibly transformed statement (fe L518, L525) or the original (fe L538).
- `ddl`, `temporary`: classification of that AST at push-or-prepare time (sm L465, L467).

## 4. Outputs and effects

No return value.
Writes, by arm:

| Arm | Lines | State written |
| --- | --- | --- |
| queue pop | sm L705, L953 | `executions` |
| `Savepoint` | sm L708-L720 | `savepoints` (push) |
| `ReleaseSavepoint` | sm L721-L731 | `savepoints` (truncate) or `unmodelled`, `dirty` |
| `Rollback { savepoint: Some }` | sm L732-L764 | `temporary` (tables, `on_commit_drop`, `failed = false`), `savepoints`, catalog overlay contents, `encrypt_config`, `unmodelled`, `local_unmodelled`, `dirty`; or `unmodelled`, `dirty` |
| `Rollback { savepoint: None }` | sm L765-L777 | `resolver`, `encrypt_config`, `savepoints`, `dirty`, `unmodelled`, `temporary.failed = true` |
| DDL / temporary | sm L778-L803 | `temporary`, catalog overlay contents, `encrypt_config`, `unmodelled` or `local_unmodelled`, `dirty` |
| tail | sm L806-L813 | `dirty`, `in_flight_ddl`, `execution_finished` notification |

## 5. Block-by-block

### 5.1 Pop (sm L705)

`pop_execution` takes the `executions` write lock, and pops only when the front is `Execute(_)` (sm L951-L953).
It returns `None` when the queue is empty or the front is `ReadyBoundary` (sm L957-L958); it never removes a boundary.
The outer pattern `Some(Some(intent))` (sm L705) means three outcomes do nothing further: no pop, and a popped `Execute(None)`.
A popped `Execute(None)` is consumed and discarded; it was pushed without an `in_flight_ddl` increment because `executes_ddl` is false for `None` (sm L513-L517).

### 5.2 `changes_schema` capture (sm L706)

`changes_schema` is `intent.ddl || (intent.temporary && statement is CreateTable)` (sm L98-L100).
It is a pure function of the stored `Intent`, the same value the pushers used to decide the increment (sm L513-L516, L531-L532).

### 5.3 `Statement::Savepoint { name }` (sm L708-L720)

Clones the catalog overlay (`SchemaWithEdits`) out of the current resolver (sm L709-L711) and pushes a `Savepoint` holding that clone, a clone of the current encrypt config (sm L715), both unmodelled flags (sm L716-L717) and `Temporary::checkpoint()` (sm L718, L158-L163).
`as_schema_with_edits().unwrap()` (sm L710) does not panic because every value stored in `self.resolver` is built with `TableResolver::new_editable` (sm L351, L399, L768; tr L26-L28) and `as_schema_with_edits` returns `Some` for that variant (tr L59-L65).
No check for an existing savepoint of the same name; duplicates are pushed and later lookups use `rposition` (sm L725, L739).
The savepoint does not record `dirty`.

### 5.4 `Statement::ReleaseSavepoint { name }` (sm L721-L731)

Finds the last savepoint whose name matches under `identifiers_equal` (sm L725, L1054-L1056) and truncates the vector to `index`, removing that savepoint and every later one (sm L727).
When no savepoint matches it calls `mark_unmodelled(false)` (sm L729), which sets `unmodelled` and `dirty` (sm L989-L990).

### 5.5 `Statement::Rollback { savepoint: Some(name) }` (sm L732-L764)

Holds the `savepoints` write lock for the whole arm (sm L736).
On a match it:

- restores the temporary layer from the checkpoint and sets `temporary.failed = false` (sm L745-L748, L165-L173);
- truncates to `index + 1`, keeping the matched savepoint (sm L749);
- overwrites the contents of the current overlay with the checkpoint (sm L750-L752), keeping the same `Arc<TableResolver>` in `self.resolver`;
- replaces `encrypt_config` (sm L753) and both unmodelled flags (sm L754-L756);
- sets `dirty = unmodelled || resolver.has_schema_changed()` (sm L757-L760), where `has_schema_changed` is "the overlay map is non-empty" (tr L49-L57, sd L40-L42).

On no match it calls `mark_unmodelled(false)` (sm L762).

### 5.6 `Statement::Rollback { savepoint: None }` (sm L765-L777)

Replaces `resolver` with a fresh editable resolver over `base` (sm L768-L770), replaces `encrypt_config` with `base_encrypt_config` (sm L771-L772), clears `savepoints` (sm L773), stores `dirty = false` (sm L774) and `unmodelled = false` (sm L775), and sets `temporary.failed = true` (sm L776).
It does not touch `local_unmodelled`, the temporary resolver's tables, `temporary.committed` or `temporary.on_commit_drop`.
The temporary tables are restored later, in `Temporary::transaction_ended`, which runs only from `ready_for_query` at Idle status (sm L453-L455, L195-L208).
`temporary.failed` is cleared in two places: `transaction_ended` (sm L207) and the rollback-to-savepoint arm (sm L747).
`changes_schema` is false for this statement (`is_schema_ddl` and `changes_only_temporary_tables` both return false for `Rollback`, sm L474-L482, L655-L662, L1123-L1134), so the tail block does not run.

No arm matches `Statement::Commit`; a confirmed commit falls to `_ => {}` (sm L804).
Commit handling is deferred to `ReadyForQuery` at Idle (`Context::became_ready`, lc L505-L508; `publish_schema_if_changed`, ctx L1718-L1734; `ready_for_query`, sm L449-L459).

### 5.7 DDL and temporary changes (sm L778-L803)

Guard: `intent.ddl || intent.temporary`, the flags stored at prepare time (sm L778).
The body does not use those flags again; it reclassifies the statement against current state:

- `split_change` re-evaluates `changes_only_temporary_tables` and `is_schema_ddl` (sm L822, L828), which read the current `encrypt_config` through `ddl_may_change_encryption` (sm L476, L657-L659, L558, L573, L600).
- `models_ddl(&statement)` is re-evaluated at sm L788 against the current `base` schema name (sm L618-L620, L624).

`split_change(&temporary, &intent.statement)` (sm L780, L821-L864) returns:

- `[DiscardTemporary]` for `DISCARD TEMP | ALL` (sm L477-L479, L824);
- `[Temporary(statement)]` for a `CREATE TEMPORARY TABLE` that is not schema DDL (sm L476, L825);
- `[]` when the statement is not schema DDL under current state (sm L828-L830);
- for `DROP TABLE`, a partition of `names` by `temporary.resolve_table(name).is_ok()` into one `Temporary(drop_tables(..))` and one `Catalog(statement with remaining names)` (sm L831-L852);
- for `ALTER TABLE` naming a table the temporary layer resolves, `Unmodelled { connection_local: true }` when `ddl_may_change_encryption` and `Temporary(statement)` otherwise (sm L853-L861);
- `[Catalog(statement)]` for everything else, including `CREATE VIEW`, `DROP VIEW` and a `CREATE TEMPORARY TABLE` that is schema DDL (sm L862).

Per change (sm L781-L801):

- `Temporary` -> `Temporary::apply` (sm L783, L177-L184): edits the temporary resolver and records `ON COMMIT DROP` names.
- `DiscardTemporary` -> `Temporary::discard` (sm L786, L186-L191): replaces the temporary tables with an empty set and clears `on_commit_drop`; `committed` and `failed` are untouched.
- `Catalog` with `models_ddl` true -> `apply_ddl` (sm L788-L790, L928-L936).
- `Catalog` otherwise -> `mark_unmodelled(local_only)` where `local_only` is true only for a temporary `CREATE TABLE` (sm L791-L797).
- `Unmodelled { connection_local }` -> `mark_unmodelled(connection_local)` (sm L798-L800).

`mark_unmodelled(true)` sets only `local_unmodelled` (sm L986-L987).
`mark_unmodelled(false)` sets `unmodelled` and `dirty` (sm L989-L990).

### 5.8 Tail (sm L806-L813)

Runs only when `changes_schema` is true.
`dirty` is overwritten with `unmodelled || self.resolver().has_schema_changed()` (sm L807-L810).
`has_schema_changed` reads only the catalog (permanent) overlay (tr L49-L57), so a change confined to the temporary layer leaves `dirty` determined by prior catalog state.
`in_flight_ddl.fetch_sub(1)` (sm L811) and `execution_finished.notify_waiters()` (sm L812) follow.
`wait_for_ddl` creates its `Notified` future before loading `in_flight_ddl` (sm L677-L681).

## 6. Callees

### 6.1 `pop_execution` (sm L950-L960), internal

Establishes: at most one entry removed, never a `ReadyBoundary`.
Does not establish: that the removed entry belongs to the completion being reported.
Paths: front is `Execute` -> pop (sm L952-L955); otherwise `None` (sm L957-L958).
The `unreachable!()` at sm L955 is guarded by the `matches!` at sm L952 under the same write guard.

### 6.2 `Intent::changes_schema` (sm L98-L100), internal

Pure; see 5.2.

### 6.3 `resolver()` / `encrypt_config()` (sm L367-L369, L387-L389), internal

Clone the current `Arc` under a read lock.
The returned resolver `Arc` aliases the live overlay, so writes through `as_schema_with_edits()` at sm L752 and inside `collect_ddl_with_column_kind` at sm L929 change what `resolver()` and `effective_resolver()` (sm L373-L378) return to later callers.

### 6.4 `TableResolver::as_schema_with_edits`, `has_schema_changed`, `resolve_table` (tr L49-L65, L96-L104), external source available

- `as_schema_with_edits` returns `None` for `ViaSchema` (tr L61); see 5.3 for why the two `unwrap()` sites (sm L710, L751) are covered.
- `has_schema_changed` is `!overlays.is_empty()` (sd L40-L42).
  No code in `schema_delta.rs` removes an overlay entry; `capture_drop_tables` and rename store `Overlay::Dropped` (sd L366, L397).
  An overlay becomes empty again only by wholesale replacement at sm L752 or by a new resolver at sm L399 or sm L768.
- `resolve_table` on the temporary resolver (`ViaSchemaWithEdits` over `Schema::new("pg_temp")`, tr L32-L34): `canonical_key` drops a qualifier equal to the schema name `pg_temp` and folds case (sd L52-L69); a miss in the overlay map falls back to the empty base schema (sd L99, sc L180-L202), which resolves nothing.
  A name qualified with any other schema does not resolve in the temporary layer.
  A bare name resolves in the temporary layer whenever a temporary table of that name is modelled, regardless of whether a catalog table of the same name exists (sm L831).

### 6.5 `Temporary::checkpoint`, `restore`, `apply`, `discard` (sm L158-L191), internal

- `checkpoint` clones the temporary `SchemaWithEdits` and `on_commit_drop` (sm L158-L163); it does not record `failed` or `committed`.
- `restore` overwrites the temporary overlay in place and `on_commit_drop` (sm L165-L173).
- `apply` calls `apply_to_temporary` (sm L178, L213-L223), which rewrites any `DROP TABLE` into a plain non-cascading drop (sm L214-L221, L245-L255) and calls `collect_ddl_with_column_kind`.
- The four `unwrap()` chains on `as_schema_with_edits()` (sm L149-L154, L166-L171) rely on `Temporary.resolver` being built by `TableResolver::new_temporary` (sm L138, tr L32-L34).

### 6.6 `split_change` (sm L821-L864), internal

See 5.7.
Not established by it: that PostgreSQL resolved the same names to the same layer.
The temporary/catalog partition uses only Proxy's model of the temporary layer (sm L831).

### 6.7 `models_ddl` -> `is_modelled_ddl`, `ddl_targets_in_model` (sm L618-L647, L1169-L1200), internal

`apply_ddl` runs only when both hold at completion time (sm L788).

- `ddl_targets_in_model` accepts every one-part name (sm L626) and a two-part name whose qualifier equals the base schema's name (sm L627-L629); longer names are rejected (sm L630).
- `is_modelled_ddl` for `Drop { Table | View }` checks only `cascade` (sm L1192-L1196); `if_exists` is not consulted.
  The module documentation lists conditional DDL as unmodelled (sm L36-L40); `CREATE TABLE IF NOT EXISTS` and `ADD COLUMN IF NOT EXISTS` are excluded (sm L1173, L1183) and `DROP ... IF EXISTS` is not.
- `is_modelled_ddl` returns `true` for every statement kind not listed (sm L1198), and `ddl_targets_in_model` returns `true` for the same (sm L645).
  Only statements wrapped in `SchemaChange::Catalog` reach the check (sm L788), and those are schema DDL under `is_schema_ddl` (sm L828).

### 6.8 `apply_ddl` (sm L928-L936), internal

Two steps with no shared lock: overlay edit (sm L929-L931), then encrypt-config copy, edit and replace (sm L933-L935).
The `bool` returned by `collect_ddl_with_column_kind` is discarded (sm L929).

`collect_ddl_with_column_kind` (sd L267-L283), external source available:

- For `ViaSchemaWithEdits` it runs `DdlCollector` over the statement (sd L272-L279).
- `CreateTable` -> `capture_create_table` replaces any existing overlay for the canonical key with a fresh table built from the declared columns (sd L306-L318).
- `AlterTable` -> `capture_alter_table`: `AddColumn`, `DropColumn`, `RenameColumn`, `RenameTable`, `ChangeColumn` edit the overlay (sd L323-L385); every other operation is a no-op (sd L387).
  Edits are skipped when the overlay entry is `Dropped` (sd L326, L337, L348, L359-L362).
- `get_overlay_mut` creates an overlay table with zero columns when the base schema does not resolve the name (sd L79-L83).
  An `ALTER TABLE ... ADD COLUMN` confirmed by PostgreSQL for a table absent from the pinned base produces an overlay table holding only the added column.
- `Drop { Table | View, cascade: false }` -> `Overlay::Dropped` per name (sd L425-L433, L392-L399).
- `CreateView` -> overlay table with native columns (sd L408-L411, L292-L304); not reached from `apply_ddl` because `is_modelled_ddl` returns false for `CreateView` (sm L1197).

`apply_encrypt_config` (sm L1071-L1120), internal:

- Keys every table by `postgres_object_name`, the last name component only (sm L1046-L1051, L1074, L1083, L1115).
  The qualifier restriction that makes this sound is the `models_ddl` guard at sm L788, not anything in `apply_encrypt_config`.
- `CreateTable`: `remove_table` then one `add_column_config` per column (sm L1073-L1079); a column gets a config only when `column_config_from_domain` returns `Some` (sm L1062).
- `AlterTable`: `AddColumn`, `DropColumn`, `RenameColumn`, `RenameTable` (sm L1086-L1104); other operations no-op (sm L1105).
- `Drop { Table | View }`: `remove_table` per name (sm L1109-L1117).
- `EncryptConfig` mutators are plain map edits (ec L48-L88).

`column_kind` (sm L1030-L1034) classifies a declared column through `eql_domains::resolve(column_domain(column))` (ed L76-L80), keyed by the unqualified, case-folded type name (sm L1022-L1027, L1046-L1051).

### 6.9 `mark_unmodelled` (sm L985-L992), internal

See 5.7.
The flags it sets are cleared by: `adopt_latest` (`unmodelled` only, sm L401), the full-rollback arm (`unmodelled` only, sm L775), and savepoint restore (both, sm L754-L756).
No other write to `local_unmodelled` exists in the file (sm L359, L755-L756, L987).

### 6.10 `tokio::sync::Notify::notify_waiters` (sm L812), external, source not read

Sent: nothing.
Depended on: every `Notified` future already created by `wait_for_ddl` (sm L677) observes this call, so a waiter that created its future and then read a non-zero `in_flight_ddl` (sm L678) is woken.
`notify_waiters` stores no permit; a waiter created after the call relies on its own load of `in_flight_ddl` at sm L678.

### 6.11 `RwLock::{read, write}().unwrap()` (throughout), std

Every lock acquisition unwraps; a poisoned lock panics the task.
No arm re-acquires a lock it already holds: the `savepoints` write guard is held across `self.temporary.write()`, `self.resolver()`, the overlay write and `self.encrypt_config.write()` (sm L736-L753), none of which touch `savepoints`.

## 7. Invariants

1. One call removes at most one queue entry and never a `ReadyBoundary` (sm L705, L952-L958).
2. `in_flight_ddl` is decremented in this function exactly when the popped intent's `changes_schema()` is true (sm L706, L806, L811), the same predicate that guarded the increment for that entry (sm L513-L516, L531-L532).
   Each entry leaves the queue once, through `pop_execution` (sm L953) or `discard_skipped_executions` (sm L966), and each of those sites applies the same predicate before decrementing (sm L942-L943, L968-L979).
3. `apply_ddl` runs only for a statement for which `models_ddl` is true at completion time (sm L788-L790).
4. Every `dirty` write in this function stores `unmodelled || catalog overlay non-empty` evaluated at that moment (sm L757-L760, L807-L810), or `true` together with `unmodelled = true` (sm L989-L990), or `false` together with a fresh empty overlay and `unmodelled = false` (sm L768-L775).
5. `self.resolver` always holds a `ViaSchemaWithEdits` resolver, so the `unwrap()` calls at sm L710 and sm L751 hold (sm L351, L399, L768; tr L26-L28, L59-L65).
6. The function contains no `await`; the pop, every state write and the decrement complete without yielding (sm L704-L815).
7. Savepoint restore keeps the matched savepoint and drops later ones (sm L749); release drops the matched savepoint and later ones (sm L727).
8. `local_unmodelled` is lowered only by savepoint restore (sm L755-L756); full rollback, `DISCARD` and `adopt_latest` leave it set (sm L765-L777, L785-L787, L392-L402).
9. A change confined to the temporary layer does not set `dirty` (sm L783, L786, L807-L810; tr L49-L57).

## 8. Assumptions

Each entry names what establishes it; "nothing found" means no line was located that enforces it.

A1. The front `Execute` entry is the execution PostgreSQL just terminated.
Established by: FIFO push order on the Frontend (sm L518-L521, L534-L536) and FIFO pop (sm L953).
Not checked against operation id or tag: neither is passed (lc L456, sm L704).

A2. Every terminal message (`CommandComplete`, `EmptyQueryResponse`, `PortalSuspended`) that reaches this function has exactly one queued `Execute` entry, and every queued `Execute` entry receives exactly one terminal message or error before its boundary.
Established by: one push per forwarded Execute (sm L518-L521) and one per sqltk-parsed statement (sm L529-L535).
For simple queries the push count is the sqltk statement count (fe L411, L480-L541, L568-L569) while the completion count is PostgreSQL's; when the Query is not rewritten the original bytes are forwarded (fe L596 onward, `Ok(None)` at fe L222).
Nothing found that compares the two counts.
A shortfall of entries is absorbed silently (`None` at sm L957-L958); a surplus of entries is removed at the next readiness boundary by `discard_skipped_executions` (sm L966-L974) without being applied.

A3. Every `ReadyForQuery` that reaches `ready_for_query` has exactly one `ReadyBoundary` queued for it, so `discard_skipped_executions` removes only entries of the batch that just ended.
Established by: `simple_query` (sm L536), `Context::sync` (lc L366), `Context::refuse_query` (lc L395).
Not established for `FunctionCall` with mapping enabled (fe L333-L335, lc L75-L77): no boundary is pushed, and `discard_skipped_executions` pops through the next boundary present in the queue (sm L966-L974).
This is upstream of `execution_succeeded`: an entry removed that way is not at the front when its completion arrives.

A4. The popped `Intent.statement` is the statement PostgreSQL executed under that portal.
Established by: `prepare` at lc L153 and `bind` at lc L238 for the normal Parse/Bind path.
Not established when: (a) the Parse is refused after lc L153 (lc L158-L159, fe L834, fe L263-L273), leaving `prepared[name]` describing a statement PostgreSQL did not receive under that name; (b) a statement or portal name is closed or deallocated, since nothing removes the intent (lc L317-L363, sm L55-L57).
Nothing found in this function that cross-checks the statement kind against the command tag.

A5. The terminal message means the statement's schema effect took place as written.
`PortalSuspended` and `EmptyQueryResponse` are applied identically to `CommandComplete` (lc L456).
A `CommandComplete` whose tag differs from the statement's own verb is applied as the statement (sm L707; tag not passed, lc L456).
Nothing found.

A6. A confirmed full `ROLLBACK` undoes every change the overlay has accumulated since the last adoption, so `dirty = false` is correct afterwards (sm L774).
Established by: nothing found.
No arm observes `COMMIT` (sm L804), so the function holds no record of whether a commit completed between the DDL that set `dirty` and the rollback that clears it within one readiness window (one simple-query message, or one extended batch before Sync).
`dirty` is next read at Idle readiness by `publish_schema_if_changed` (ctx L1719-L1726).

A7. `temporary.failed = true` set by the full-rollback arm (sm L776) is consumed before any later transaction's temporary-table changes are confirmed.
Established by: `transaction_ended` at Idle readiness (sm L453-L455, L195-L208) and savepoint restore (sm L747).
Nothing found that clears it when a new transaction starts within the same readiness window; `transaction_ended` with `failed` set restores `committed` and discards every temporary change applied since (sm L196-L200).

A8. The `savepoints` vector mirrors PostgreSQL's savepoint stack for the current transaction.
Established by: push at sm L712, truncation at sm L727 and sm L749, clear at sm L773.
Also cleared by `adopt_latest` (sm L400), reachable from `before_statement` when `transaction_active` is false (sm L438-L446), from `ready_for_query` at Idle (sm L456-L458) and from `publication_succeeded` (sm L427-L430).
`transaction_active` is written by pushes (sm L514, L527) and overwritten by each readiness status (sm L451-L452); an Idle readiness for an earlier batch overwrites the `true` stored by a later, already queued batch.
A miss in the vector is handled by `mark_unmodelled(false)` (sm L729, L762).

A9. A bare table name refers to the modelled schema, and a bare name the temporary layer resolves refers to the temporary table.
Established by: `ddl_targets_in_model` accepting all one-part names (sm L626) and `is_temporary` (sm L831).
Nothing found that reads or tracks the connection's `search_path`.

A10. The pinned `base` schema contains every catalog table PostgreSQL resolves for a modelled `ALTER TABLE`.
Established by: nothing found in this function; `get_overlay_mut` fabricates an empty table on a miss (sd L79-L83).

A11. The classification inputs read at completion time (`encrypt_config` for `split_change`, `base` for `models_ddl`) are the ones relevant to the statement PostgreSQL executed.
Established by: single-task execution per connection (drv L222-L231 comment; no `await` in the function).
The stored `Intent.ddl` / `Intent.temporary` flags gate the arm (sm L778) and the tail (sm L706, L806) while the body reclassifies (sm L822, L828, L788); the two can differ when the encrypt config changed between Parse and completion.
When the reclassification yields `[]` (sm L828-L830) the tail still runs for an intent whose stored `changes_schema()` is true.

A12. Frontend and Backend handlers of one connection do not run this function concurrently with a queue writer.
Established by: the driver's single-task intermediary (drv L222-L231 comment) and the system-context statement that every field is per-connection.
Within the function each lock is taken and released separately (for example sm L779 then sm L783), so the function itself does not provide atomicity across its steps.

A13. `notify_waiters` wakes every `wait_for_ddl` future created before the call.
Established by: tokio `Notify` semantics, source not read (sm L677-L681, L812).

## 9. Shared state

Read:

- `executions` (sm L951), `resolver` (sm L709, L750, L808), `encrypt_config` (sm L715, L933), `base` (sm L769, L624), `base_encrypt_config` (sm L772), `unmodelled` (sm L716, L808), `local_unmodelled` (sm L717), `savepoints` (sm L722, L736), `temporary` (sm L718, L779).

Written:

- `executions` (sm L953), `savepoints` (sm L712, L727, L749, L773), `resolver` (sm L768), catalog overlay contents (sm L752, L929), `encrypt_config` (sm L753, L771, L935), `unmodelled` (sm L754, L775, L989), `local_unmodelled` (sm L755, L987), `dirty` (sm L757, L774, L807, L990), `temporary` (sm L746-L747, L776, L783, L786), `in_flight_ddl` (sm L811), `execution_finished` (sm L812).

Not touched: `store` (`CommittedSchemaStore`), `base`, `base_encrypt_config`, `prepared`, `portals`, `transaction_active`.
The function requests no publication; `dirty` is the only channel to `publish_schema_if_changed` (ctx L1719-L1726, sm L405-L412).

Downstream readers of what it writes:

- `dirty`: `needs_publication`, `has_local_changes`, `requires_publication_before_statement`, `before_statement` (sm L405-L446).
- `unmodelled` / `local_unmodelled`: `has_unmodelled_ddl` (sm L433-L435) -> `Context::ensure_schema_modelled` (ctx L1408-L1413).
- `resolver` / `temporary.resolver`: `effective_resolver` (sm L373-L378) -> `Context::get_table_resolver` (ctx L1351-L1353), and `batch_resolvers` (sm L872-L916).
- `encrypt_config`: `Context::is_passthrough` (ctx L1753-L1755), `has_encrypted_columns` (ctx L1748-L1751), the `ColumnMapper` constructions (ctx L1762-L1785), and `ddl_may_change_encryption` (sm L558, L573, L600).
- `in_flight_ddl`: `wait_for_ddl` (sm L675-L683), `ddl_in_flight` (sm L686-L688) -> driver drain loop (drv L220, L258), `before_statement` (sm L442).

## 10. Open questions

1. Unclear whether pg-proto delivers a terminal message (`CommandComplete`, `EmptyQueryResponse`, `PortalSuspended`) with `operation == None`; need to inspect pg-proto 0.12.0 (source not on disk).
   The answer decides whether the `operation.is_some()` gate at be L244-L246 can skip a pop that the other three call sites perform.
2. Unclear how many terminal messages PostgreSQL emits for a simple Query relative to sqltk's statement count for the same text; need to inspect `SqlParser::parse_statements` and its handling of empty statements and of statement kinds that PostgreSQL splits differently.
3. Unclear which SQL spellings sqltk parses into `Statement::Rollback { savepoint: None }`, `Statement::Commit`, `Statement::Savepoint` and `Statement::ReleaseSavepoint` (for example `ABORT`, `END`, `ROLLBACK PREPARED`, `PREPARE TRANSACTION`); need to inspect the sqltk parser.
4. Unclear whether a Bind naming a statement whose Parse was refused after lc L153 is itself refused before `bind_schema_statement` (lc L238); need to inspect `bind_settings_statement` and `Frontend::bind_handler`, and what `extended_parse_error` (fe L272) sends to PostgreSQL under that statement name.
5. Unclear whether a pipelined batch can be queued behind a `FunctionCall` while mapping is enabled; need to inspect pg-proto's sequencing of `FunctionCall` and the driver loop to determine whether the `ReadyForQuery` of a FunctionCall can arrive while later `Execute` entries are queued.
6. Unclear what PostgreSQL does with the temporary-layer and overlay expectations when `ROLLBACK AND CHAIN` succeeds: the arm sets `temporary.failed = true` (sm L776) and readiness status stays non-idle; need to inspect whether any test covers a chained transaction.
7. Unclear whether `Intent.modelled` (sm L88, L466) is intended to be read at completion; no reader exists in the file.
8. Unclear whether `Schema.name` for the pinned base is always the single schema that bare names resolve to; need to inspect `SchemaManager` load code in `mgr` for how `Schema::name` and multi-schema tables are populated (sc L177-L179 describes one-part names matching a table in any loaded schema).
9. Unclear whether `tokio::sync::Notify::notify_waiters` covers a `Notified` future that has been created but not yet polled; need to inspect the tokio version pinned in `Cargo.lock`.
