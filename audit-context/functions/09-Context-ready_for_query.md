# Context::ready_for_query

- File: `packages/cipherstash-proxy/src/postgresql/context/mod.rs`
- Lines: L931-L1015
- Signature: `fn ready_for_query(&self, status: TransactionStatus, boundary: Option<OperationId>) -> Result<(), Error>`
- Kind: private synchronous method on `Context<T>`; no `await`; takes the `protocol_state` write lock once for the sweep and again (through a callee) once per readiness-completed simple-query execution.
- Format note: `ANALYSIS_FORMAT.md` was not present on disk (the `{baseDir}` placeholder did not resolve), so this file uses the section order of the sibling files in `audit-context/functions/`.

Abbreviations used for citations:

- `ctx` = `packages/cipherstash-proxy/src/postgresql/context/mod.rs`
- `lc` = `packages/cipherstash-proxy/src/postgresql/context/lifecycle.rs`
- `sp` = `packages/cipherstash-proxy/src/postgresql/context/settings_protocol.rs`
- `st` = `packages/cipherstash-proxy/src/postgresql/context/settings.rs`
- `ps` = `packages/cipherstash-proxy/src/postgresql/context/prepared_statements.rs`
- `po` = `packages/cipherstash-proxy/src/postgresql/context/portal.rs`
- `be` = `packages/cipherstash-proxy/src/postgresql/middleware/backend.rs`
- `mw` = `packages/cipherstash-proxy/src/postgresql/middleware/mod.rs`
- `pg` = `packages/cipherstash-proxy/src/postgresql/mod.rs`

## 1. Purpose

`ready_for_query` is the per-connection readiness sweep run when PostgreSQL sends `ReadyForQuery`.
It does four things, in this order, under one write lock (ctx L937-L1004):

1. Settles the confirmed `ConnectionSettings` against the reported transaction status (ctx L941).
2. Tells `SettingsProtocol` that readiness arrived, which clears two flags unconditionally and, when a boundary operation is supplied, retires `pending`, `boundaries` and `journal` entries with id `<= boundary` (ctx L942-L944, sp L453-L482).
3. When a boundary is supplied and status is `Idle`, removes portals and suspended executions whose owning operation id is `<= boundary` (ctx L949-L982).
4. When a boundary is supplied, removes every `operations` entry with id `<= boundary` for any status (ctx L984-L1002), then releases statement-metrics scopes that nothing references any more (ctx L1003).

After the lock is released it records execution-duration histograms, closes the unnamed portal of a simple query that completes at readiness, and records the released metrics scopes (ctx L1005-L1013).

When `boundary` is `None` the function returns `Ok(())` at ctx L946, after steps 1 and 2 have already written.

## 2. Reachability and caller contract

### 2.1 Call chain

Production call site: `Context::became_ready` (lc L505).
Test-only wrapper: `test_ready_for_query` (ctx L1986-L1992) and direct calls in the `ctx` test module (for example ctx L2913, L2938, L2982, L3007).

`became_ready` is called from three places in `Backend::intercept_backend`, each on `BackendMessage::ReadyForQuery`:

- passthrough branch (be L199-L201),
- discard branch (be L220-L225), which also clears `discard_execution` first (be L221),
- normal branch (be L354-L360).

`operation` reaches `ready_for_query` unchanged from pg-proto's `Option<OperationId>` argument to `backend_operation` (mw L65-L76, lc L500-L505), wrapped by `From<pg_proto::OperationId>` (pg L30-L34).
`status` is the status byte decoded by pg-proto from PostgreSQL's `ReadyForQuery` (be L199, L220, L354).

### 2.2 What runs after it in the caller

`became_ready` uses `?` on `ready_for_query` (lc L505).
On `Ok`, it awaits `publish_schema_if_changed` when status is `Idle` (lc L506-L508) and then calls `schema_ready_for_query` (lc L509-L517).
On `Err` from `ready_for_query`, neither of those runs (lc L505).
On `Err` from `publish_schema_if_changed`, `schema_ready_for_query` does not run although the protocol-state sweep has already completed (lc L505-L509).

### 2.3 A path on which a ReadyForQuery does not reach this function

In the normal branch, a non-`DataRow` message that arrives while `encrypted_rows` is non-empty first triggers `flush_encrypted_rows` (be L230-L235).
When that flush fails, `intercept_backend` returns at be L247 before reaching the `ReadyForQuery` arm at be L354.
`report_outcome` at be L245 does nothing for `ReadyForQuery` (be L95-L111).
So a `ReadyForQuery` that is itself the message triggering a failed flush is not passed to `became_ready`.
The code does not prevent `encrypted_rows` from being non-empty when `ReadyForQuery` arrives; that condition is excluded only by PostgreSQL sending a terminal message between the last `DataRow` and `ReadyForQuery`.

### 2.4 Serialisation with the Frontend

`Frontend` and `Backend` are two fields of one `CipherStashMiddleware` (mw L15-L18) and both trait methods take `&mut self` (mw L54-L76).
`ready_for_query` contains no `await` (ctx L931-L1015).
So no Frontend handler of this connection runs between the lock release at ctx L1004 and the re-acquisitions at ctx L1179-L1182.

## 3. Inputs

- `status: TransactionStatus` (pg-proto type): compared only against `Idle` (ctx L944, L949); `InTransaction` and `FailedTransaction` are treated identically by this function.
- `boundary: Option<OperationId>`: the operation pg-proto correlated with this `ReadyForQuery`. `None` means uncorrelated.
- `self.protocol_state: Arc<RwLock<ConnectionProtocolState>>` (ctx L67): the only state mutated.
- `self.config` and `self.client_id`: read by `record_finished_metrics_scope` (ctx L606-L607, L622).

`OperationId` ordering is the derived `Ord` over `OperationIdInner::Protocol(pg_proto::OperationId)` (pg L20-L28).
Every `<=` and `>` in this function and in `SettingsProtocol::ready` delegates to pg-proto's `Ord` for its operation id.

## 4. Outputs

- `Ok(())` on the uncorrelated path (ctx L946) and at the end (ctx L1014).
- `Err(ContextError::ProtocolStateUnavailable)` when the `RwLock` is poisoned at ctx L937-L940, or at ctx L1179-L1182 through `close_portal_if_current` (ctx L1008).
- No value derived from the sweep is returned to the caller.

## 5. Block-by-block

### 5.1 Lock acquisition (ctx L936-L940)

Write lock on `protocol_state`; poison maps to `ProtocolStateUnavailable`.
The guard lives until the end of the block expression at ctx L1004, or until the early return at ctx L946.

### 5.2 `state.settings.ready_for_query(status)` (ctx L941)

Runs for every call, before `boundary` is examined.
Callee body (st L372-L379):

- status not `Idle`: no write. `failed`, `transaction_start`, `savepoints` and transaction overrides are all left as they were.
- status `Idle` and `self.failed`: `restore_transaction()` (st L375), which replaces `keyset` and `unsafe_disable_mapping` with the `transaction_start` snapshot when one exists and does nothing when it is `None` (st L381-L386).
- status `Idle`, always: `end_transaction()` (st L377), which clears both transaction overrides, sets `transaction_start = None`, `failed = false`, and clears `savepoints` (st L407-L413).

Writers of the two inputs to the restore decision:

- `failed` is set by `ConnectionSettings::execution_failed` (st L368-L370), called unconditionally at the end of `restore_failed_preparation` (ctx L264), including when `operation` is `None` (ctx L216-L220, sp L337-L339).
  It is cleared by `end_transaction` (st L411) and by a matched `RollbackTo` (st L343).
- `transaction_start` is captured lazily by `command_succeeded` (st L318-L322), which runs only when `SettingsProtocol::succeeded` returns a command (ctx L1581-L1582).
  It is cleared by `restore_transaction` (`take`, st L382) and `end_transaction` (st L410).

The `failed` flag is connection-wide, not operation-scoped (st L297).
The restore at st L374-L376 therefore depends on any error response seen since the last `end_transaction`, not on an error inside the boundary's batch.

### 5.3 `state.settings_protocol.ready(boundary, idle)` (ctx L942-L944)

Runs for every call, before the `None` early return.
Callee body (sp L453-L482):

- Unconditional: `settings_since_boundary = false` (sp L455) and `discarding_until_sync = false` (sp L457).
- Only when `boundary` is `Some(operation)`:
  - `pending.retain(|id, _| *id > operation)` (sp L459). Entries removed here are dropped with whatever `commands` they still hold; no command is applied.
  - `boundaries.retain(|id| *id > operation)` (sp L460). There is no check that `operation` is a member of `boundaries`.
  - `journal.retain` keeps ids `> operation`; removed entries contribute `metadata.metrics()` to the return value (sp L461-L468). Removed entries are not rolled back; the live objects stay as they are.
  - Only when `idle`: `portals.retain(|_, bound| bound.operation > operation)` (sp L470), and every surviving journal mutation whose `previous` is `Previous::Portal(Some(bound))` with `bound.operation <= operation` has `previous` set to `Portal(None)`, its metadata metrics retired and its metadata reset (sp L471-L478).
- Returns the collected `SessionId`s (sp L481). With `boundary == None` the vector is empty (sp L454, L458).

`prepared` is not touched by `ready` (sp L453-L482).

### 5.4 Uncorrelated early return (ctx L945-L947)

`let Some(boundary) = boundary else { return Ok(()); }`.
State already written on this path:

- `ConnectionSettings`: full `Idle` settlement when status is `Idle` (st L373-L378).
- `SettingsProtocol.settings_since_boundary = false`, `discarding_until_sync = false` (sp L455, L457).

State not touched on this path: `pending`, `boundaries`, `journal`, `SettingsProtocol.portals`, `operations`, `portals`, `portal_operations`, `suspended_executions`, `statement_metrics`.
`candidates` is empty here (sp L454, L458), so no metrics id is lost by the return.
No histogram is recorded on this path.

### 5.5 Idle-only portal sweep (ctx L949-L965)

Selection is over `portal_operations`: names whose `owner.operation <= boundary` (ctx L950-L956).
For each selected name, `portal_operations[name]` and `portals[name]` are removed, and the portal's `session_id()` becomes a metrics candidate (ctx L957-L965, po L75-L80).
A `portals` entry with no `portal_operations` entry is not selected.
`install_portal` writes both maps together (ctx L205-L212); `restore_failed_preparation` writes them independently from two optional snapshot fields (ctx L256-L261).

`PortalOwner.operation` is the operation passed to `install_portal` (ctx L207-L208): the Bind operation on the extended path (lc L275), the Query operation on the simple path (lc L60, L377).

### 5.6 Idle-only suspended sweep (ctx L967-L981)

Selection is over `suspended_executions`: names whose stored operation `<= boundary` (ctx L967-L973).
The stored operation is the Execute operation that suspended (ctx L379-L380).
Removed entries contribute `execute.session_id()` as a metrics candidate (ctx L974-L981).
No execution-duration histogram is recorded for these: they are not pushed to `executions` (ctx L948, L974-L981).

When status is not `Idle`, sections 5.5 and 5.6 are skipped (ctx L949); portals and suspended executions survive readiness in `InTransaction` and `FailedTransaction` alike.

### 5.7 Operation sweep, all statuses (ctx L984-L1002)

Every key of `operations` with id `<= boundary` is removed (ctx L984-L993).
Only the `execute` field of the removed `OperationContext` is used (ctx L994).
`describe`, `error_response`, `preparation` and `deallocations` (ctx L115-L123) are dropped without inspection.
For a removed `execute`:

- its metrics context is cloned from `statement_metrics` (not removed) (ctx L996-L998),
- its session id becomes a candidate (ctx L999),
- the pair `(execute, metrics)` is queued for recording (ctx L1000).

An `execute` is still present at this point in two situations:

- a simple-query execution created with `until_readiness()` (ctx L670), which `finish_execution` deliberately leaves in place on `Completed` (ctx L345-L358);
- an Execute whose terminal message never reached `finish_execution` (which `take`s it, ctx L359).

Both are recorded identically at ctx L1006.

Session ids carried by `Preparation::Parse { metrics }` and `Preparation::Bind { metrics }` (lc L13-L23) in a removed operation are not added to `candidates` (ctx L991-L1001).

### 5.8 `state.take_unreferenced_metrics(candidates)` (ctx L1003)

For each candidate, removes the `statement_metrics` entry unless `metrics_referenced` is true (ctx L324-L335).
`metrics_referenced` checks, in the post-sweep state: journal snapshots (sp L155-L159), `prepared_statements` (ps L130-L134), `portals`, `operations[*].execute`, `suspended_executions` (ctx L305-L322).
A candidate that appears twice is removed on first sight; the second `remove` yields `None` (ctx L331).

### 5.9 Recording, lock released (ctx L1005-L1013)

For each finished execution:

- `record_execution_duration` (ctx L1006) emits one histogram sample with labels from the cloned metrics or `("unknown","unknown","false","false")` (ctx L742-L770).
- When `completes_at_readiness && name.is_empty()`, `close_portal_if_current(&execute.name, execute.portal.as_ref())?` (ctx L1007-L1008).

`close_portal_if_current` (ctx L1173-L1203) re-takes the write lock, returns `Ok(())` without change unless `portals[name]` is the same `Arc` as `expected` (`Arc::ptr_eq`, ctx L1183-L1190), otherwise removes `portal_operations[name]` and `portals[name]` and releases the portal's metrics scope if unreferenced (ctx L1191-L1201).
At `Idle` the unnamed portal owned by the query operation has already been removed at ctx L957-L964, so this call is a no-op there.
At non-`Idle` status this call is what removes the simple query's unnamed portal.

The `?` at ctx L1008 exits the loop on a poisoned lock.
In that case the remaining `finished_executions` and every `finished_metrics_scopes` entry, already removed from `statement_metrics` at ctx L1003, are dropped without being recorded (ctx L1005-L1013).

Finally each released metrics scope goes to `record_finished_metrics_scope` (ctx L1011-L1013), which returns without recording when `records_statement_metrics` is false (ctx L576-L578), otherwise emits a histogram and, when slow-statement logging is enabled and the threshold exceeded, a counter and a `warn!` that includes `keyset_id = self.keyset_identifier()` read at log time (ctx L606-L635, L1638-L1644).
The keyset logged is therefore the post-settlement value written at ctx L941, not the value in effect while the statement ran.

## 6. Callees

| Callee | Kind | What the caller depends on |
| --- | --- | --- |
| `ConnectionSettings::ready_for_query` (st L372-L379) | internal | At `Idle`: transaction overrides dropped, snapshot restored iff `failed`, `failed`/`transaction_start`/`savepoints` cleared. At other statuses: nothing. |
| `SettingsProtocol::ready` (sp L453-L482) | internal | Clears both flags always; retires pending/boundaries/journal `<= boundary` only for `Some`; portal table and journal `previous` neutralisation only for `Some` and `idle`; returns metrics ids of retired journal snapshots. |
| `Portal::session_id` (po L75-L80) | internal | Pure field read. |
| `ExecuteContext::session_id` (ctx L109-L111) | internal | Pure field read. |
| `ConnectionProtocolState::take_unreferenced_metrics` / `metrics_referenced` (ctx L305-L335) | internal | Removes a scope only when no journal snapshot, statement, portal, in-flight execute or suspended execute still names it. |
| `Context::record_execution_duration` (ctx L737-L771) | internal, calls `metrics::histogram!` | No state change. |
| `Context::close_portal_if_current` (ctx L1173-L1203) | internal | Identity check by `Arc::ptr_eq` so a portal installed later under the same name is left alone. |
| `Context::record_finished_metrics_scope` (ctx L574-L638) | internal, calls `metrics` macros, `tracing::warn!`, `keyset_identifier` (read lock) | No protocol-state mutation. |
| `Ord for pg_proto::OperationId` (via pg L20-L28) | external black box | Total order consistent with the order operations were issued on this connection. |

## 7. Invariants

1. `ConnectionSettings::ready_for_query(status)` runs on every call, correlated or not (ctx L941 precedes ctx L945).
2. `SettingsProtocol.settings_since_boundary` and `discarding_until_sync` are false after every call, correlated or not (ctx L942-L944, sp L455, L457).
3. With `boundary == None`, no collection in `ConnectionProtocolState` other than `settings` fields changes size (ctx L945-L947, sp L458).
4. With `boundary == Some(b)`, on `Ok` return no key `<= b` remains in `operations`, `SettingsProtocol.pending`, `SettingsProtocol.journal`, and no element `<= b` remains in `SettingsProtocol.boundaries` (ctx L984-L993, sp L459-L468).
5. With `boundary == Some(b)` and `Idle`, no `portal_operations` entry has `owner.operation <= b`, no `suspended_executions` entry has operation `<= b`, and no `SettingsProtocol.portals` entry has `bound.operation <= b` at the point the lock is released (ctx L950-L981, sp L470).
6. With status other than `Idle`, `portals`, `portal_operations`, `suspended_executions` and `SettingsProtocol.portals` are not modified inside the locked block (ctx L949, sp L469); the only later portal removal is the `ptr_eq`-guarded unnamed-portal close (ctx L1007-L1008).
7. A `statement_metrics` entry is removed by this function only when its id was a candidate and `metrics_referenced` was false in the post-sweep state (ctx L1003, L329-L332; ctx L1197 for the unnamed portal).
8. `prepared_statements` and `SettingsProtocol.prepared` are not modified (ctx L931-L1015, sp L453-L482).
9. No protocol-state lock is held while metrics or log macros run (block ends ctx L1004; ctx L1198 ends the callee's block before ctx L1199-L1201).

## 8. Assumptions

1. `boundary` is the id of the request (Sync, simple Query, FunctionCall) that this `ReadyForQuery` answers.
   Established by: pg-proto correlation, source not on disk. Nothing in this function or in `SettingsProtocol::ready` checks membership in `boundaries` (sp L460).
2. Every operation with id `<= boundary` has already received all the backend messages it will ever receive.
   Established by: the comment-level premise "Responses arrive in order" (sp L493) plus pg-proto's id ordering; nothing found in this crate that verifies it. The sweep at ctx L984-L993 removes those operations unconditionally.
3. `OperationId` order equals frontend issue order on this connection.
   Established by: pg-proto (black box); pg L20-L28 only derives `Ord`.
4. An uncorrelated `ReadyForQuery` with status `Idle` is a legitimate signal that the transaction ended and that extended-protocol discard ended.
   Established by: nothing found. ctx L941 and sp L455-L457 apply the settlement without a boundary, while `pending` entries of in-flight operations are left queued (sp L458).
5. `failed == true` at `Idle` means the failure belongs to the transaction whose `transaction_start` snapshot is held.
   Established by: nothing found. `failed` is set for every error response including uncorrelated ones (ctx L264) and is cleared only at st L343 and st L411.
6. `status` reflects PostgreSQL's real transaction state.
   Established by: upstream PostgreSQL via pg-proto decoding; not cross-checked against the commands applied through `command_succeeded` (st L317-L366).
7. Every `ReadyForQuery` PostgreSQL sends reaches this function.
   Established on be L199-L201, L220-L225, L354-L360; not established on the failed-flush return at be L236-L248 (section 2.3).
8. A `pending` entry with id `<= boundary` holds no command that PostgreSQL executed but has not yet confirmed.
   Established by: nothing found in this function; sp L459 drops remaining `commands` unapplied. `succeeded` pops one command per `CommandComplete` (sp L288-L289).
9. `portals` and `portal_operations` hold the same key set, so sweeping by `portal_operations` sweeps every portal.
   Established by: `install_portal` (ctx L205-L212), `close` (lc L341-L347), `close_portal_if_current` (ctx L1191-L1196), DISCARD ALL retain (ctx L1595-L1596). Not established by `restore_failed_preparation`, which inserts each map from an independent `Option` (ctx L256-L261).
10. A journal snapshot that survives readiness (id `> boundary`) does not carry a Context-level portal, portal owner or suspended execution that this sweep just removed.
    Established only for mutations whose `previous` is `Portal(Some(bound))` with `bound.operation <= boundary` (sp L471-L478). Not established when `previous` is `Portal(None)` while `metadata.portal` is `Some`: `bind` takes `previous` from `SettingsProtocol.portals` (sp L200) and `metadata` from the Context-level maps (ctx L1471, L282-L289), and the simple-query path installs a Context portal without any `SettingsProtocol.portals` entry (lc L377, sp L262-L286).
11. An operation removed at ctx L993 has no `describe`, `error_response`, `preparation` or `deallocations` that still matter.
    Established by: nothing found in this function; ctx L994 keeps only `execute`.
12. No other code runs on this connection's state between ctx L1004 and ctx L1179.
    Established by: `&mut self` on both middleware entrypoints (mw L54-L76) and the absence of `await` in ctx L931-L1015.

## 9. Shared state touched

Written:

- `ConnectionProtocolState.settings` (ctx L941).
- `SettingsProtocol.{pending, boundaries, journal, portals, settings_since_boundary, discarding_until_sync}` (ctx L942-L944).
- `ConnectionProtocolState.{portal_operations, portals}` (ctx L958-L963, and L1191-L1196 via callee).
- `ConnectionProtocolState.suspended_executions` (ctx L976-L979).
- `ConnectionProtocolState.operations` (ctx L991-L993).
- `ConnectionProtocolState.statement_metrics` (ctx L1003, L1197).
- Process-global metrics recorder (ctx L763-L770, L596-L603, L612).

Read only: `Context.config`, `Context.client_id`.

Not touched: `prepared_statements`, `SettingsProtocol.prepared`, `SchemaMiddleware` (handled afterwards by the caller, lc L509-L517), `Backend` buffers.

## 10. Open questions

1. Unclear which operation id pg-proto attaches to `ReadyForQuery`, and in which situations it passes `None` (startup readiness, readiness after an asynchronous error, readiness after a Sync the middleware suppressed); need to inspect pg-proto 0.12.0 source, which is not on disk.
2. Unclear whether pg-proto's `OperationId` ordering is strictly monotonic in frontend arrival order and whether it wraps; need to inspect pg-proto 0.12.0.
3. Unclear whether a surviving journal entry with `previous == Portal(None)` and `metadata.portal == Some(..)` can be replayed by `SettingsProtocol::failed` (sp L365-L430) into `restore_failed_preparation` (ctx L252-L261) after this sweep removed that portal; need to trace `failed` for a pipelined Bind that follows a simple Query whose `Idle` readiness arrives between them.
4. Unclear whether an operation can reach readiness with `preparation == Some(Preparation::Bind { metrics: Some(_) })` still set, which would leave that metrics id outside `candidates`; need to inspect every Frontend exit between `begin_bind` (lc L231-L259) and `finish_bind`/`refuse_bind` (lc L261-L304).
5. Unclear what `Backend::decryption_failure` emits and whether it later causes a synthetic readiness; need to inspect `be` `decryption_failure` to settle section 2.3.
6. Unclear whether `Frontend` reads `discarding_extended_messages` (ctx L1550-L1557) in a way that depends on the flag being cleared by an uncorrelated readiness; need to inspect `middleware/frontend.rs` call sites.
7. Unclear whether the `mapping_disabled` configuration paths, which skip `SettingsProtocol::close` and `execute` (lc L307-L310, L323-L334), leave `SettingsProtocol.portals` and Context `portals` with different key sets at readiness; need to inspect `unmapped_*` callers in `middleware/frontend.rs`.
