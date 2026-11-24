# Context::settings_execution_succeeded

- File: `packages/cipherstash-proxy/src/postgresql/context/mod.rs`
- Lines: L1568-L1606
- Signature: `fn settings_execution_succeeded(&self, operation: Option<OperationId>, tag: &str) -> Result<(), Error>`
- Kind: private method on `Context<S>`, synchronous, takes the `protocol_state` write lock, no I/O apart from metrics recording after the lock is released.
- Format note: `ANALYSIS_FORMAT.md` was not present on disk (the `{baseDir}` placeholder did not resolve), so this file uses the section order of the files in `audit-context-frontend/functions/`: purpose, reachability, inputs, outputs, block-by-block, callees, invariants, assumptions, shared state, open questions.

Abbreviations used for citations:

- `ctx` = `packages/cipherstash-proxy/src/postgresql/context/mod.rs`
- `lc` = `packages/cipherstash-proxy/src/postgresql/context/lifecycle.rs`
- `sp` = `packages/cipherstash-proxy/src/postgresql/context/settings_protocol.rs`
- `st` = `packages/cipherstash-proxy/src/postgresql/context/settings.rs`
- `ps` = `packages/cipherstash-proxy/src/postgresql/context/prepared_statements.rs`
- `be` = `packages/cipherstash-proxy/src/postgresql/middleware/backend.rs`
- `fe` = `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`
- `mw` = `packages/cipherstash-proxy/src/postgresql/middleware/mod.rs`
- `drv` = `packages/cipherstash-proxy/src/postgresql/driver.rs`
- `pg` = `packages/cipherstash-proxy/src/postgresql/mod.rs`

## 1. Purpose

Applies one PostgreSQL `CommandComplete` to the connection's confirmed settings.
It pops the next queued `SettingsCommand` for the correlated operation from `SettingsProtocol.pending` (ctx L1581, sp L287-L289) and feeds it, with the command tag, to `ConnectionSettings::command_succeeded` (ctx L1582, st L317-L366).
When the popped command is `SettingsCommand::Change(SettingChange::DiscardAll)` it additionally prunes three `ConnectionProtocolState` maps (`prepared_statements`, `portals`, `portal_operations`) down to the names `SettingsProtocol` still holds after its own DISCARD pruning (ctx L1583-L1596), then releases every metrics scope nothing references any more (ctx L1597-L1598, L1602-L1604).

This is the only non-test writer of `ConnectionSettings` keyset and mapping values.
`command_succeeded` has three call sites: this function (ctx L1582), a projection over a clone (sp L57), and two projections over clones in `may_change_effective` (st L198, L205); `execution_succeeded` (st L310) is `#[cfg(test)]`.

## 2. Reachability and caller contract

Single call site: `Context::execution_completed` (lc L454), reached only for `ExecutionEnd::Command(tag)` (lc L453).
`EmptyQueryResponse` and `PortalSuspended` map to `ExecutionEnd::Empty` and `ExecutionEnd::Suspended` (be L101-L108) and do not reach this function.

`execution_completed` is called from `Backend::report_outcome` (be L96-L100), which has four call sites in `intercept_backend`:

1. Passthrough connection, `discarded = false` (be L187-L188).
   `is_passthrough` is `encrypt_config().is_empty() || config.mapping_disabled()` (ctx L1753-L1755).
2. `discard_execution` mode, `discarded = true` (be L218-L219); the backend message is then suppressed toward the client (be L227).
3. Flush of buffered rows failed, `discarded = true`, only when `operation.is_some()` (be L244-L246).
4. Normal path, `discarded = false` (be L254), after any buffered encrypted rows for the previous rows were flushed (be L230-L252).

`execution_completed` calls this function before it consults `discarded` (lc L453-L458).
The settings command is therefore applied on all four paths, including the two where the client does not receive the `CommandComplete`.

Ordering inside one `CommandComplete`:

- `complete_deallocation` runs first when the tag is `DEALLOCATE`, `DEALLOCATE ALL` or `DISCARD ALL` and the operation is `Some` (be L163-L170, L763-L772).
- Buffered rows are flushed and decrypted with the pre-change settings (be L230-L252; doc comment lc L444).
- This function runs (lc L454).
- `report_schema_execution_succeeded` runs (lc L456), then `discard_operation` or `finish_execution` (lc L457-L468).

An `Err` from this function propagates through `?` at lc L454, so the schema report and `finish_execution` for that `CommandComplete` do not run.

Serialisation with the Frontend: `frontend_operation` and `backend_operation` both take `&mut self` on the one `CipherStashMiddleware` that owns both adapters (mw L15-L18, L54-L76).
A Frontend handler suspended at an `.await` therefore holds the exclusive borrow and this function does not run inside it.
The driver comment describes the intermediary as single-task (drv L226-L228).

Pipelining: the driver drains backend outcomes before accepting further frontend work only when the forwarded message was a `Query`, a `Sync` or an expanded message and `schema_ddl_in_flight() || settings_in_flight()` (drv L214-L221, L258-L262).
`settings_in_flight` is `pending.values().any(|p| p.barrier)` (ctx L1621-L1628, sp L521-L523).
A `DISCARD ALL` is a barrier only when applying it changes the effective keyset or mapping flag (st L189-L201).
With default effective settings a `DISCARD ALL` is not a barrier, and later frontend messages can be processed before this function runs for it.
The `operation > discard` comparisons in `SettingsProtocol::succeeded` (sp L300-L302) exist for that case.

## 3. Inputs

| Input | Source | Controlled by |
|---|---|---|
| `operation: Option<OperationId>` | pg-proto's correlation for the backend message (mw L70, L74), wrapped at pg L30-L34 | pg-proto (black box) |
| `tag: &str` | `String::from_utf8_lossy` of the `CommandComplete` payload (be L98) | upstream PostgreSQL |
| `state.settings_protocol.pending[operation]` | Frontend: `SettingsProtocol::execute` (sp L252-L259), `simple_query` (sp L276-L283), `sync` (sp L443-L450) | client SQL as parsed by Proxy |
| `state.settings`, `prepared_statements`, `portals`, `portal_operations`, `statement_metrics` | `ConnectionProtocolState` (ctx L154-L163) | earlier Frontend and Backend handlers of this connection |

## 4. Outputs and effects

- Returns `Ok(())` or `Err(ContextError::ProtocolStateUnavailable)` when the `RwLock` is poisoned (ctx L1576-L1579). `RwLock` is `std::sync::RwLock` (ctx L38, L67).
- Mutates `SettingsProtocol.pending`, `settings_since_boundary`, and on DISCARD ALL `prepared`, `portals`, `journal` (sp L287-L325).
- Mutates `ConnectionSettings` (st L317-L366).
- On DISCARD ALL mutates `prepared_statements`, `portals`, `portal_operations`, `statement_metrics` (ctx L1594-L1598).
- Emits metrics for released scopes after the lock is dropped (ctx L1601-L1604).

## 5. Block-by-block

### 5.1 Uncorrelated completion (ctx L1573-L1575)

`operation == None` returns `Ok(())` with no lock taken and no state change.
A `CommandComplete` pg-proto does not correlate applies no settings command.
The failure counterpart does not have this early return: `settings_execution_failed` passes `None` into `restore_failed_preparation`, which sets `settings.failed` unconditionally (ctx L1608-L1613, L264; `failed(None)` returns empty at sp L337-L339).

### 5.2 Lock (ctx L1576-L1579)

Write lock on `protocol_state`; held until the explicit `drop(state)` at ctx L1601.
Nothing between L1579 and L1601 awaits or calls outside `ConnectionProtocolState`.

### 5.3 Join to the queued command (ctx L1581)

`SettingsProtocol::succeeded(operation)` returns `Option<SettingsCommand>`.
`None` skips every remaining effect; the function then drops the lock and returns `Ok(())` with an empty `finished`.
`None` arises on three paths, read from sp L287-L325:

- No `Pending` for the operation (sp L288, `?`).
- A `Pending` whose `commands` deque is empty: one inserted by `sync` (sp L446), one cleared by `failed` (sp L358-L360), or a simple-query `Pending` that has already yielded one command per parsed statement (sp L289).
- Never for a non-empty deque: `pop_front` returns `Some`.

The join is positional.
The n-th `CommandComplete` correlated to an operation receives the n-th command Proxy derived from the n-th statement it parsed (sp L269-L275, L289).
`succeeded` does not receive the tag and does not compare the command kind with it.

On the empty-deque paths `succeeded` still executes sp L290-L293 before returning `None`: it ORs `pending.barrier` into `settings_since_boundary` and removes a non-`until_ready` entry.

### 5.4 Apply (ctx L1582)

`state.settings.command_succeeded(&statement, tag)`.
Read from st L317-L366:

- Every call first captures `transaction_start` if it is `None` (st L318-L322), including for `SettingsCommand::Other`.
- `Commit` and `Rollback`: `restore_transaction()` only when `command_tag == "ROLLBACK"` exactly, then `end_transaction()` (st L324-L329). This is the only use of `tag`.
- `Savepoint`, `RollbackTo`, `Release`: savepoint stack edits; an unknown savepoint name is a silent no-op (st L330-L350).
- `Change(Keyset | Mapping | ResetKeyset | ResetMapping | ResetAll)`: scoped assignment (st L351-L363).
- `Change(DiscardAll)`: `self.reset()`, which is `*self = Self::default()` (st L356, L416-L418). This also clears `transaction_start`, `failed` and `savepoints`, including the snapshot captured at st L318-L322 in the same call.
- `Other`, `Begin`: nothing beyond the snapshot capture (st L364).

`command_succeeded` returns `()` and has no failing path.

### 5.5 DISCARD ALL pruning (ctx L1583-L1599)

Entered only when the popped command is `Change(DiscardAll)`.
The condition tests the queued command, not the tag.

By this point `succeeded` has already pruned `SettingsProtocol` (sp L294-L323):

- `prepared` keeps the unnamed statement and any statement whose `Prepared.operation > operation` (sp L299-L300).
- `portals` keeps a portal whose `Bound.operation > operation` (sp L301).
- `journal` keeps entries with id `> operation` (sp L302); dropped entries' `MetadataSnapshot`s are discarded without their metrics ids being returned.
- Remaining journal entries whose `previous` refers to an object with operation `<= operation` have `previous` set to `None` and `metadata` reset to default; a `Previous::Statement` for the unnamed name is exempt (sp L303-L322).

The caller then mirrors membership by name:

- `prepared_statements.retain(|name| settings_protocol.has_statement(name))` (ctx L1594; `retain` at ps L91-L93; `has_statement` is `prepared.contains_key` at sp L515-L517).
- `portals.retain(|name, _| settings_protocol.has_portal(name))` (ctx L1595; sp L518-L520).
- `portal_operations.retain(|name, _| settings_protocol.has_portal(name))` (ctx L1596).

The retain predicates do not look at the operation id recorded in the pruned store (`PreparedStatement.parse` at ps L12, `PortalOwner.operation` at ctx L167-L170).
Survival in the three maps is decided entirely by whether `SettingsProtocol` has an entry under the same name.

Not pruned here: `suspended_executions` (ctx L159), `operations` (ctx L157), and anything in `SchemaMiddleware`.

Metrics sweep: every key of `statement_metrics` is a candidate (ctx L1597).
`take_unreferenced_metrics` removes each scope for which `metrics_referenced` is false (ctx L324-L335).
`metrics_referenced` counts references from `settings_protocol.journal` snapshots (sp L155-L159), `prepared_statements` (ps L130-L134), `portals`, `operations[*].execute`, and `suspended_executions` (ctx L305-L322).
It does not count `operations[*].preparation` (`Preparation::Parse { metrics }` / `Preparation::Bind { metrics }`, lc L13-L23).
This full sweep is what releases scopes whose last reference was a journal snapshot dropped at sp L302 or reset at sp L320, since `succeeded` returns only the command.

### 5.6 Unlock and record (ctx L1601-L1605)

`drop(state)` precedes `record_finished_metrics_scope` for each released scope.
`record_finished_metrics_scope` returns without recording when `records_statement_metrics` is false (ctx L574-L578); otherwise it records histograms (ctx L579-L599).

## 6. Callees

### 6.1 `SettingsProtocol::succeeded` (sp L287-L325) — internal

Depended on for: the positional join, removal of the extended-protocol `Pending`, and the DISCARD pruning of `prepared` / `portals` / `journal` that the caller's three `retain` calls copy.
All paths walked in 5.3 and 5.5.
Simple-query `Pending` entries (`until_ready: true`) are never removed here; `ready` removes them (sp L459).

### 6.2 `ConnectionSettings::command_succeeded` (st L317-L366) — internal

Depended on for: all confirmed-settings mutation.
Paths walked in 5.4.

### 6.3 `SettingsProtocol::has_statement` / `has_portal` (sp L515-L520) — internal

Pure `contains_key` lookups.

### 6.4 `PreparedStatements::retain` (ps L91-L93) — internal

Removes whole entries (statement, metrics scope id, parse op, deallocation) for names the predicate rejects.

### 6.5 `ConnectionProtocolState::take_unreferenced_metrics` (ctx L324-L335) and `metrics_referenced` (ctx L305-L322) — internal

Depended on for: not removing a scope that a live object still uses.
Reference set listed in 5.5.

### 6.6 `Context::record_finished_metrics_scope` (ctx L574-L599) — internal, calls the `metrics` crate macros

Called without the lock held.

### 6.7 `RwLock::write` — std

Poisoning maps to `ContextError::ProtocolStateUnavailable` (ctx L1579).

## 7. Invariants

1. A settings command reaches `ConnectionSettings` only together with a correlated `CommandComplete`: the sole call site is guarded by `ExecutionEnd::Command` (lc L453-L454) and by `operation` being `Some` (ctx L1573-L1575).
2. At most one queued command is consumed per call (`pop_front`, sp L289).
3. After a DISCARD ALL branch, every name in `prepared_statements`, `portals` and `portal_operations` is also a key of `SettingsProtocol.prepared` / `SettingsProtocol.portals` respectively (ctx L1594-L1596). The converse is not established by this function.
4. After `succeeded` on DISCARD ALL, every named entry in `SettingsProtocol.prepared` and every entry in `SettingsProtocol.portals` has `operation > ` the DISCARD operation, and the unnamed statement is kept regardless (sp L299-L301).
5. After `command_succeeded(DiscardAll)`, `ConnectionSettings` equals `Default`: no keyset, mapping enabled, no `transaction_start`, `failed == false`, no savepoints (st L356, L416-L418, L290-L299).
6. The `protocol_state` write lock is not held while metrics are recorded (ctx L1601-L1604).
7. A metrics scope referenced by a journal snapshot, a prepared statement, a portal, an operation's `execute`, or a suspended execution survives the sweep (ctx L305-L322, L330).
8. This function does not run while a Frontend handler of the same connection is suspended (`&mut self` on both entrypoints, mw L54-L76).

## 8. Assumptions

A1. The n-th `CommandComplete` pg-proto correlates to an operation corresponds to the n-th statement Proxy parsed for it.
Established by: nothing found. `succeeded` pops by position (sp L289) and neither it nor this function compares the tag with the command. The separate deallocation queue does compare (`matches!(deallocation, Deallocation::AllStatements) != all`, ctx L877-L887), and on mismatch keeps statements; this function has no equivalent, so a popped `DiscardAll` resets settings and prunes regardless of the tag.

A2. `operation` identifies the frontend message that caused this `CommandComplete`.
Established by: pg-proto 0.12.0 (source not on disk). Proxy only wraps the id (pg L30-L34).

A3. `OperationId` ordering equals the order in which PostgreSQL processes frontend messages, so `operation > discard` means "prepared or bound after the DISCARD ALL".
Established by: `derive(Ord)` over `pg_proto::OperationId` (pg L20-L26); the ordering semantics of the inner id are pg-proto's. The comment at sp L493 states the reliance for `deallocated`.

A4. `tag` is exactly the command tag with no trailing NUL when compared with `"ROLLBACK"` (st L325).
Established by: nothing found in Proxy. `is_deallocation` and `removes_all_statements` strip an optional trailing `\0` from the same bytes (be L764, L770), while `execution_completed` passes the lossy string unmodified (be L98).

A5. Every name in `prepared_statements` that PostgreSQL still holds after DISCARD ALL has an entry in `SettingsProtocol.prepared`, so that the by-name retain at ctx L1594 removes only what DISCARD ALL removed.
Established by: paired Frontend writes on the mapped path — `begin_parse` creates the `prepared_statements` entry (lc L107-L112), `prepare_parse` creates the `SettingsProtocol.prepared` entry (lc L159, ctx L1440-L1454, sp L168-L177), and a failure between them goes to `refuse_parse`, which removes the `prepared_statements` entry (fe L263-L270, lc L208-L229, ctx L784-L796). Rollback keeps the pair aligned because `previous` and `metadata` are captured together (lc L99-L102, sp L145-L153) and cleared together (sp L316-L320, L385-L391, L507-L508).
Not established for: an entry created outside that pairing. `complete_deallocation` deliberately keeps a statement with no recorded Parse (ps L109-L126); the retain at ctx L1594 applies no such exception.

A6. Every portal in `portals` that must survive DISCARD ALL has an entry in `SettingsProtocol.portals`.
Established by: `begin_bind` calling `bind_settings_statement` before `finish_bind` installs the portal (lc L237, L275), but only on the path where `SettingsProtocol.prepared` contains the statement; `SettingsProtocol::bind` returns `Ok(())` without inserting when it does not (sp L210-L212).
Not established for the unnamed portal of a simple Query: `register_query` installs `portals[""]` and `portal_operations[""]` (lc L377, ctx L1060-L1068, L205-L212) and `SettingsProtocol::simple_query` inserts nothing into `SettingsProtocol.portals` (sp L262-L286).
A simple-query DISCARD ALL therefore removes `portals[""]` at ctx L1595, whichever operation installed it, unless a later extended Bind of the unnamed portal is present in `SettingsProtocol.portals`.
The running operation keeps its own `Arc<Portal>` in `ExecuteContext` (ctx L669-L671), and the Backend reads portals through `get_portal_from_execute` (ctx L1290-L1306; be L479, L736).

A7. No metrics scope that is still needed is unreferenced at the moment of the sweep.
Established by: invariant 8 plus each Frontend handler leaving its scope referenced by `prepared_statements`, a portal, or `operations[*].execute` before returning. `operations[*].preparation` is not in the reference set (ctx L305-L322), so a `Preparation` left in place by a handler that returned early would not protect its scope. Not verified for every early-return path of every handler.

A8. PostgreSQL does not roll back a `DISCARD ALL` it has confirmed.
Established by: nothing found in Proxy. `reset()` discards `transaction_start` and `savepoints` (st L416-L418), so `ready_for_query` / a later `ROLLBACK` has no snapshot from before the DISCARD to restore (st L381-L386).

A9. The Frontend's two independent derivations of "this operation runs DISCARD ALL" agree: `SettingChange::parse` for the settings queue (st L34-L41) and `to_deallocation` for the deallocation queue (fe L1458-L1475, fe L421-L427, ctx L698-L704).
Established by: both match `Statement::Discard { object_type: DiscardObject::ALL }`; no shared code enforces it.

A10. On a connection with operator-level `mapping_disabled`, no `Pending` holding a command exists, so the function is a no-op.
Established by: the Frontend's early branch calls `unmapped_query` / `unmapped_bind` / `unmapped_parse` and never `simple_query` (fe L189-L211, lc L42-L70), and `Context::execute` skips `execute_settings_portal` (lc L307-L310). `unmapped_sync` can insert only an empty-command `Pending` (sp L443-L450).

## 9. Shared state touched

- `ConnectionProtocolState.settings` — written (ctx L1582).
- `ConnectionProtocolState.settings_protocol` — `pending`, `settings_since_boundary`, `prepared`, `portals`, `journal` written (sp L287-L325).
- `ConnectionProtocolState.prepared_statements`, `portals`, `portal_operations`, `statement_metrics` — written on DISCARD ALL (ctx L1594-L1598).
- `ConnectionProtocolState.operations`, `suspended_executions` — read only, through `metrics_referenced` (ctx L312-L321).
- Process-wide metrics recorder — written through `record_finished_metrics_scope` (ctx L596-L599).

All of the above except the metrics recorder are per-connection and reached through this connection's `Context` clones (`protocol_state: Arc<RwLock<..>>`, ctx L67).

## 10. Open questions

- Unclear whether pg-proto can deliver more or fewer `CommandComplete` messages for one operation than `SqlParser::parse_statements` produced statements; need to inspect pg-proto's correlation and the cases where sqltk and PostgreSQL split a Query differently.
- Unclear whether the `CommandComplete` payload pg-proto hands over carries a trailing NUL; need to inspect pg-proto's `BackendMessage::CommandComplete` decoding (affects st L325).
- Unclear whether pg-proto ever supplies `operation == None` for a `CommandComplete`; need to inspect pg-proto's backend correlation.
- Unclear whether `SchemaMiddleware`'s own `prepared` / `portals` maps are pruned on DISCARD ALL; need to inspect `SchemaMiddleware::execution_succeeded` in `proxy/schema/middleware.rs` (its `SchemaChange::DiscardTemporary` handling at L785 and L896).
- Unclear what later consumers do with a `suspended_executions` entry whose portal was removed at ctx L1595 before the next Idle readiness; need to inspect `set_execute_for_portal` (ctx L676-L706) callers and `ready_for_query` (ctx L967-L981).
- Unclear whether `describe_handler` or any other Frontend path reads `portals[""]` by name for a pipelined simple Query after an earlier DISCARD ALL removed it; need to inspect `get_portal` callers in `fe`.
- Unclear whether any Frontend handler can return with `operations[op].preparation` still set while its metrics scope is otherwise unreferenced; need to inspect the `?` exits of `parse_handler` and `bind_handler` and whether pg-proto can drop a handler future mid-await.
- Unclear whether PostgreSQL can report `CommandComplete` for DISCARD ALL and later abort the enclosing implicit transaction of an extended-protocol pipeline; need to inspect PostgreSQL's `PreventInTransactionBlock` behaviour for pipelines.
