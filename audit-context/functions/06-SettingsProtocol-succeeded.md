# SettingsProtocol::succeeded

- File: `packages/cipherstash-proxy/src/postgresql/context/settings_protocol.rs`
- Lines: L287-L325
- Signature: `pub fn succeeded(&mut self, operation: OperationId) -> Option<SettingsCommand>`
- Kind: synchronous method on `SettingsProtocol`, no I/O, no `await`, mutates five of the seven fields of `self`.
- Format note: `ANALYSIS_FORMAT.md` was not present on disk (the `{baseDir}` placeholder did not resolve), so this file uses the section order of the files in `audit-context-frontend/functions/`: purpose, reachability, inputs, outputs, block-by-block, callees, invariants, assumptions, shared state, open questions.

Abbreviations used for citations:

- `sp` = `packages/cipherstash-proxy/src/postgresql/context/settings_protocol.rs`
- `st` = `packages/cipherstash-proxy/src/postgresql/context/settings.rs`
- `ctx` = `packages/cipherstash-proxy/src/postgresql/context/mod.rs`
- `lc` = `packages/cipherstash-proxy/src/postgresql/context/lifecycle.rs`
- `ps` = `packages/cipherstash-proxy/src/postgresql/context/prepared_statements.rs`
- `be` = `packages/cipherstash-proxy/src/postgresql/middleware/backend.rs`
- `fe` = `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`
- `mw` = `packages/cipherstash-proxy/src/postgresql/middleware/mod.rs`
- `pg` = `packages/cipherstash-proxy/src/postgresql/mod.rs`

## 1. Purpose

`succeeded` is the point where a settings command recorded by the Frontend is handed back for application to the confirmed `ConnectionSettings`.
It takes the next queued `SettingsCommand` for `operation` off the front of that operation's `Pending.commands` queue and returns it (sp L288-L289, L324).
The selection is positional: the function receives no command tag and no statement identity, only the operation id (sp L287).
When the popped command is `SettingsCommand::Change(SettingChange::DiscardAll)` it additionally removes, from its own tables, every named statement, every portal and every journal entry whose recorded operation id is not greater than `operation`, and neutralises the rollback snapshot of later journal entries that point at removed objects (sp L294-L323).

## 2. Reachability and caller contract

### 2.1 Call chain

There is one production call site: `Context::settings_execution_succeeded` (ctx L1581).
Two test call sites exist (sp L660, L934).

The production chain is:

1. pg-proto delivers each backend message to `CipherStashMiddleware::backend_operation` with `operation: Option<pg_proto::OperationId>` (mw L65-L76), converted with `Into` (mw L74, pg L30-L34).
2. `Backend::intercept` -> `Backend::intercept_backend` (be L148-L160).
3. `Backend::report_outcome` (be L89-L112), called from four places in `intercept_backend`: the passthrough branch (be L188), the discard branch (be L219), the failed-flush branch when `operation.is_some()` (be L244-L246), and the normal path (be L254).
4. For `BackendMessage::CommandComplete(tag)` only, `report_outcome` calls `Context::execution_completed(operation, ExecutionEnd::Command(&String::from_utf8_lossy(tag)), discarded)` (be L96-L100).
5. `execution_completed` calls `settings_execution_succeeded(operation, tag)` only for `ExecutionEnd::Command` (lc L453-L455).
   `ExecutionEnd::Empty` (from `EmptyQueryResponse`, be L101-L104) and `ExecutionEnd::Suspended` (from `PortalSuspended`, be L105-L108) do not reach `succeeded`.
6. `settings_execution_succeeded` returns `Ok(())` without calling `succeeded` when `operation` is `None` (ctx L1573-L1575); otherwise it takes the `protocol_state` write lock (ctx L1576-L1579) and calls `state.settings_protocol.succeeded(operation)` (ctx L1581).

### 2.2 What the caller does not gate on

- `discarded`: `settings_execution_succeeded` runs before `discarded` is consulted (lc L453-L459), so a `CommandComplete` that the Backend suppresses from the client still pops and applies a command.
  The test at ctx L2477-L2483 records this as intended ("A suppressed PostgreSQL success still activates its settings").
- `is_passthrough`: the passthrough branch calls `report_outcome` (be L187-L188).
- The command tag: the tag is passed to `settings_execution_succeeded` (lc L454) and from there only to `ConnectionSettings::command_succeeded` (ctx L1582); it is not passed to `succeeded` (ctx L1581, sp L287).

### 2.3 What runs before `succeeded` on the same message

For a `CommandComplete` with `Some(operation)` whose tag is `DEALLOCATE`, `DEALLOCATE ALL` or `DISCARD ALL` (be L763-L766), `Context::complete_deallocation` runs first (be L163-L170), ahead of every `report_outcome` call site.
`complete_deallocation` pops the operation's `deallocations` queue (ctx L874), compares the queued kind against the tag (ctx L877-L887), and for each affected name calls `SettingsProtocol::deallocated(operation, name)` (ctx L891-L896) and then `PreparedStatements::deallocate` (ctx L897).
`deallocated` removes `prepared[name]` when its `operation < operation` (sp L486-L492) and clears `previous` and `metadata` on the first later journal mutation for that statement name (sp L496-L509).
So when `succeeded` runs its `DISCARD ALL` branch, a tag-driven pass over named statements has already happened on the same message, selected by a different queue (`OperationContext.deallocations`, written at fe L421-L427 and ctx L698-L704) and checked against the tag.

### 2.4 Writers of the queue `succeeded` consumes

`self.pending` entries are created in three places:

- `SettingsProtocol::execute` (sp L252-L259): one command, cloned from `bound.prepared.command`, `until_ready: false`, `barrier` computed from the confirmed settings at Execute time (sp L250-L251).
  Reached from `Context::execute` -> `execute_settings_portal` (lc L311, ctx L1509-L1511) only when `config.mapping_disabled()` is false (lc L307-L310).
  No entry is created when the portal is absent from `self.portals` (sp L239-L241) or when the settings check refuses (sp L242-L249).
- `SettingsProtocol::simple_query` (sp L276-L283): one command per element of `statements`, `until_ready: true`.
  Reached from `Context::register_query` -> `execute_simple_settings_statements` (lc L380, ctx L1528) with `forwarded_statements` (fe L568-L569), which receives exactly one push per parsed statement (fe L480-L541: L518 or L525 on the `Map` arm, L538 on the `Forward` arm).
- `SettingsProtocol::sync` (sp L443-L450): empty `commands`, `barrier: true`, `until_ready: true`, keyed by the Sync (or refused Query, lc L394) operation id, only when `settings_since_boundary || in_flight()` (sp L442).

`pending` entries are removed or altered by `succeeded` itself (sp L292), by `failed` (sp L354-L364; a skipped entry with `until_ready && barrier` is kept with `commands` cleared, every other skipped entry is removed), and by `ready` (sp L459, every id not greater than the readiness operation).

Paths that create no `pending` entry for the operation, so that `succeeded` returns at L288:

- Operator-level mapping disabled: `unmapped_query` (lc L58-L63) and the early return in `Context::execute` (lc L307-L310).
- `refuse_query` (lc L384-L397) unless its `sync` call inserts the empty barrier entry (sp L442-L450).
- `refuse_execute` (lc L399-L406).
- Execute of a portal that `SettingsProtocol::bind` did not record because the statement name was absent from `self.prepared` (sp L210-L212).

## 3. Inputs

- `operation: OperationId` (sp L287).
  A `Copy + Ord` wrapper around `pg_proto::OperationId` (pg L20-L34).
  Its value is the correlation pg-proto attached to the backend message (mw L70-L74).
  The function uses it as a map key (sp L288, L292) and as an ordering threshold (sp L300-L302, L309, L313).
- `self.pending`, `self.prepared`, `self.portals`, `self.journal`, `self.settings_since_boundary` (sp L129-L134).
  All were written earlier by Frontend-side methods on the same `ConnectionProtocolState` (section 2.4; `prepare` sp L168-L183, `bind` sp L200-L229, `start_prepare` sp L145-L153, `close` sp L531-L543).

The function does not read `self.boundaries` or `self.discarding_until_sync`.

## 4. Outputs

- Return value `Option<SettingsCommand>`:
  - `None` when there is no `pending` entry for `operation` (sp L288, the `?`).
  - `None` when the entry exists and its queue is empty (sp L289).
  - `Some(command)` otherwise, the former front of the queue (sp L289, L324).
- State changes on every path that passes L288: `settings_since_boundary` (sp L290), `pending` (sp L289, L292).
- State changes on the `DiscardAll` path: `prepared` (sp L299-L300), `portals` (sp L301), `journal` membership (sp L302), `journal` values (sp L303-L322).

The function returns no error and has no panic site: there is no indexing, `unwrap` or arithmetic in L287-L325.

## 5. Block-by-block

### 5.1 Lookup, L288

`let pending = self.pending.get_mut(&operation)?;`

A missing entry returns `None` immediately.
Nothing is mutated on this path, including `settings_since_boundary`.
The caller then skips `command_succeeded` (ctx L1581-L1582), so `ConnectionSettings` is untouched, including the lazy `transaction_start` capture that `command_succeeded` performs on every call (st L318-L322).

### 5.2 Pop, L289

`let command = pending.commands.pop_front();`

One element leaves the queue per call.
The n-th call for an operation returns the n-th command queued for it.
Nothing in the function relates the popped command to the `CommandComplete` that triggered the call; the tag is not an argument (sp L287).
An empty queue yields `None` and the function continues to L290.

Queue-empty-with-entry-present arises in three ways visible in this file:

- A `sync`-created entry, which starts empty (sp L446).
- A `simple_query` entry after `failed` cleared it (sp L358-L360).
- A `simple_query` entry (`until_ready: true`, so never removed at L291-L293) after as many calls as it had commands.

### 5.3 Barrier accumulation, L290

`self.settings_since_boundary |= pending.barrier;`

This runs whenever the entry exists, including when `command` is `None`.
`pending.barrier` was computed on the Frontend path against the settings confirmed at that time (sp L251 for Execute; sp L268-L275 via `project_settings_effects` sp L47-L61 for a simple query, where each statement is evaluated against a projection that assumes every earlier statement of the batch succeeded with `success_tag()`, sp L56-L57).
For a simple query the flag is the OR across the whole batch (sp L272), so the first completion of a batch sets `settings_since_boundary` even when the barrier-raising statement is a later one.
`settings_since_boundary` is read only by `sync` (sp L442) and reset only by `ready` (sp L455).

### 5.4 Entry removal, L291-L293

The entry is removed only when `until_ready` is false and the queue is now empty.
`until_ready` is false only for `execute`-created entries (sp L257), which hold exactly one command (sp L255), so an Execute entry is removed by its first call.
A second `CommandComplete` under the same Execute operation then takes the L288 return.
`simple_query` and `sync` entries (`until_ready: true`, sp L281, L448) stay until `failed` (sp L354-L364) or `ready` (sp L459).
While they stay, an entry with `barrier: true` keeps `in_flight()` true (sp L521-L523).

An Execute entry whose execution ends with `PortalSuspended` or `EmptyQueryResponse` is not popped (section 2.1 step 5) and stays, with its command still queued, until `failed` or `ready`.
A later Execute of the same portal has a different operation id and inserts a separate entry (sp L252).

### 5.5 DISCARD ALL test, L294-L297

The branch is selected by the popped command alone.
`SettingChange::DiscardAll` is produced only by `SettingChange::parse` for `Statement::Discard { object_type: DiscardObject::ALL }` (st L34-L41), reached through `SettingsCommand::from_statement` (st L183-L185) in `prepare` (sp L167) and `project_settings_effects` (sp L55).
`DISCARD PLANS` and `DISCARD TEMP` do not produce it (test st L871-L876).

### 5.6 Named statements, L299-L300

`self.prepared.retain(|name, prepared| name.is_empty() || prepared.operation > operation);`

- The unnamed statement is kept unconditionally (comment sp L298).
- A named statement is kept only when its `Prepared.operation`, the Parse operation id set at sp L171, is strictly greater than the completing operation.

### 5.7 Portals, L301

`self.portals.retain(|_, bound| bound.operation > operation);`

Every portal, named or unnamed, whose Bind operation id (sp L224) is not greater than `operation` is removed.
A kept `Bound` carries its own clone of the `Prepared` it was bound from (sp L210, L225), so a portal bound after the discard keeps the settings command and `EffectiveSettings` of a statement that L300 removed.

### 5.8 Journal membership, L302

`self.journal.retain(|id, _| *id > operation);`

Journal entries keyed at or below `operation` are dropped.
Their `MetadataSnapshot` is dropped with them and no `SessionId` is returned from it.
`ready` performs the equivalent drop and collects `mutation.metadata.metrics()` into its return value (sp L461-L468); `succeeded` has no such output (sp L287, L324).
The caller compensates by offering every key of `statement_metrics` to `take_unreferenced_metrics` (ctx L1597-L1598), which removes each scope that `metrics_referenced` no longer finds (ctx L305-L335).

### 5.9 Journal rollback snapshots, L303-L322

For each remaining mutation (all keyed above `operation`):

- `Previous::Statement(previous)`: expired when the mutation's name is non-empty and `previous` is `Some` with `operation <= operation` (sp L305-L310).
  A mutation on the unnamed statement is never expired by this arm (sp L306).
- `Previous::Portal(previous)`: expired when `previous` is `Some` with `bound.operation <= operation` (sp L311-L313), for any name.
- An expired mutation has `previous` set to the `None` of the same variant (sp L316-L319) and `metadata` replaced by `MetadataSnapshot::default()` (sp L320).

`metadata` is cleared only together with `previous`.
A mutation whose `previous` is already `None` is not expired (`is_some_and` is false, sp L307-L309, L311-L313) and keeps whatever `metadata` it holds.
`previous` and `metadata` come from two different sources at creation time: `previous` from this struct's `prepared`/`portals` (sp L148, L200, L532-L533), `metadata` from `ConnectionProtocolState::statement_snapshot` / `portal_snapshot` over the Context tables (lc L99-L102, L324-L333, ctx L1471, ctx L271-L289).
`failed` later hands the retained `metadata` back in `RestoreMetadata` (sp L425-L429) and `restore_failed_preparation` writes it into the Context tables (ctx L221-L262).

### 5.10 Return, L324

The popped command, or `None`.

## 6. Callees

`succeeded` calls only standard-library collection methods (`BTreeMap::get_mut`, `VecDeque::pop_front`, `BTreeMap::remove`, `HashMap::retain`, `BTreeMap::retain`, `BTreeMap::values_mut`), `Name::is_empty`, and `MetadataSnapshot::default`.
None has a failure path that the function depends on.

The dependencies that matter run the other way: the single caller builds on what `succeeded` leaves behind.

### 6.1 `Context::settings_execution_succeeded` (ctx L1568-L1606), consumer

- Applies the returned command with `state.settings.command_succeeded(&statement, tag)` (ctx L1582).
  `command_succeeded` uses the tag only in the `Commit | Rollback` arm, comparing it with `"ROLLBACK"` (st L324-L329); every `Change(..)` arm applies its change regardless of the tag (st L351-L363).
  The caller therefore depends on `succeeded` returning the command that belongs to this `CommandComplete`.
- On `DiscardAll`, prunes the Context tables by membership in the tables `succeeded` just pruned: `prepared_statements.retain(|name| settings_protocol.has_statement(name))` (ctx L1594, ps L91-L93, sp L515-L517), `portals.retain(|name, _| settings_protocol.has_portal(name))` and the same for `portal_operations` (ctx L1595-L1596, sp L518-L520).
  The caller applies no operation-id comparison of its own; survival of a Context statement or portal is decided entirely by whether `SettingsProtocol` holds an entry under that name after L299-L301.
  `suspended_executions` is not pruned in this block (ctx L1587-L1598).
- Then sweeps `statement_metrics` (ctx L1597-L1598) and records the removed scopes after releasing the lock (ctx L1601-L1604).

The write lock is held from before the call until `drop(state)` (ctx L1576-L1601), so the Frontend observes the pop, the settings application and both prunes as one step.

### 6.2 `ConnectionSettings::command_succeeded` (st L317-L366), downstream of the return value

For `DiscardAll` it calls `self.reset()` (st L356), which replaces the whole struct with its default (st L416-L418): keyset, mapping flag, `transaction_start`, `failed` and `savepoints`.

### 6.3 pg-proto, external black box

`operation` originates in pg-proto (mw L70-L74); the crate source is not on disk.
What `succeeded` takes from it without a local check is listed in section 8.

## 7. Invariants

1. At most one command leaves `pending[operation].commands` per call (sp L289).
2. A `Some` return is always a command previously queued under the same operation id (sp L288-L289); the function never synthesises a command.
3. A missing `pending` entry leaves all state unchanged (sp L288).
4. An `execute`-created entry is removed by the call that empties it (sp L291-L293, with sp L255, L257).
5. A `simple_query`- or `sync`-created entry is never removed by `succeeded` (sp L291, with sp L281, L448).
6. `settings_since_boundary` is only ever raised here, never lowered (sp L290).
7. The unnamed statement in `self.prepared` survives `DISCARD ALL` (sp L300).
8. After the `DiscardAll` branch, every entry in `prepared` (named), `portals` and `journal` has an operation id strictly greater than `operation` (sp L300-L302).
9. After the `DiscardAll` branch, no remaining journal mutation holds a `previous` portal with `operation <= operation`, and no remaining named-statement mutation holds a `previous` statement with `operation <= operation` (sp L303-L322).
10. The `DiscardAll` branch runs only when the popped command is `DiscardAll` (sp L294-L297); it does not run on a `None` pop.

## 8. Assumptions

Each entry names where it is established, or "nothing found".

1. The n-th `CommandComplete` correlated to a simple-query operation is the completion of the n-th statement in `forwarded_statements`.
   The queue is built with one command per sqltk-parsed statement (sp L269-L275, fe L480-L541).
   `succeeded` pops by position (sp L289) and receives no tag (sp L287).
   Established by: nothing found.
   The line that would establish it is sp L289 or ctx L1581-L1582; by contrast `complete_deallocation` compares its queued item with the tag and clears its queue on mismatch (ctx L877-L887).
   Related facts: `EmptyQueryResponse` consumes no command (lc L453); a completion after the queue is exhausted returns `None` and applies nothing (sp L289, ctx L1581); a batch that produces fewer completions than commands leaves the remainder queued until `failed` or `ready` (sp L291, L354-L364, L459).

2. PostgreSQL's statement boundaries for a Query string equal the boundaries sqltk produced in `SqlParser::parse_statements`.
   The forwarded bytes are the client's original text when nothing is rewritten (fe L220-L222, "don't change the bytes"), while the queue is derived from the parsed AST (fe L411, L568-L569).
   Established by: nothing found.

3. An extended-protocol Execute yields at most one `CommandComplete` under its operation id, and that completion is for the command cloned at Bind time into `bound.prepared.command`.
   The entry holds one command (sp L250-L255) taken from the `Prepared` snapshot cloned in `bind` (sp L210, L225).
   Established by: the remove-on-empty at sp L291-L293 bounds the effect of a second completion to a `None` return; the identity of the command with what PostgreSQL executed is established by nothing found in this file.

4. pg-proto correlates each `CommandComplete` with the operation id of the Query or Execute that caused it, and with no other id (for example a Sync id).
   A `sync`-created entry under a Sync id has an empty queue and `barrier: true` (sp L443-L450); a call with that id returns `None` and sets `settings_since_boundary` (sp L289-L290).
   Established by: pg-proto, source not available.

5. `OperationId` ordering equals the order in which the Frontend processed the corresponding client messages, so that `> operation` means "created by a message after the discarding one".
   Used at sp L300, L301, L302, L309, L313.
   Established by: `derive(Ord)` over `pg_proto::OperationId` (pg L20-L25) gives a total order; that the order is monotonic in message arrival is established by pg-proto, source not available.

6. `Prepared.operation` is the Parse operation id and `Bound.operation` is the Bind operation id.
   Established by: sp L171 and sp L224, the only constructors.

7. Every Context statement that must outlive `DISCARD ALL` has an entry of the same name in `SettingsProtocol.prepared` after sp L300, and every Context portal that must outlive it has one in `SettingsProtocol.portals` after sp L301.
   The caller's prune is by name membership only (ctx L1594-L1596).
   Established by: partly.
   For statements, `begin_parse` writes the Context entry (lc L109-L112) and `prepare_parse` writes the `SettingsProtocol` entry later in the same handler (lc L159); a failure between the two goes to `refuse_parse`, which removes the Context entry (lc L227).
   For portals it is not established: `SettingsProtocol::bind` records no portal when the statement name is absent from `self.prepared` (sp L210-L212) while `finish_bind` still installs the Context portal (lc L275), and a simple Query installs the unnamed Context portal through `add_portal` (lc L377) with no `SettingsProtocol.portals` counterpart (sp L262-L286 writes none).
   Such Context portals are removed at ctx L1595-L1596 whatever their operation id.

8. The unnamed statement entry in the Context table survives `DISCARD ALL` because `SettingsProtocol.prepared` holds an unnamed entry.
   sp L300 keeps an unnamed entry only when one exists; ctx L1594 removes the Context unnamed statement when `has_statement("")` is false.
   Established by: lc L159 on the mapped Parse path; nothing found for a Context unnamed statement that exists without a `SettingsProtocol` counterpart.

9. A journal mutation whose `previous` is `None` carries a `metadata` snapshot that is safe to restore after the discard.
   Such a mutation is not expired (sp L307-L309, L311-L313) and its `metadata` is not cleared (sp L315-L321), while the Context entries of the same name are removed by the caller when the name is absent from `SettingsProtocol` (ctx L1594-L1596).
   Established by: nothing found; it holds only when `previous == None` implies an empty `metadata`, which depends on the two tables agreeing at mutation-creation time (sp L148 with lc L99-L102; sp L200 with ctx L1471).

10. Journal entries keyed at or below `operation` hold nothing that needs restoring or reporting when dropped at sp L302.
    Established by: the caller's full sweep of `statement_metrics` (ctx L1597-L1598) covers metrics scopes; nothing found for the other snapshot fields (`statement`, `portal`, `portal_operation`, `suspended`, sp L88-L96), which are dropped without being written back.

11. `pending.barrier`, computed on the Frontend path from the settings confirmed at that moment (sp L251, sp L269-L275), still describes the command when PostgreSQL confirms it.
    Established by: nothing found in `succeeded`; the function ORs the stored flag without recomputation (sp L290).

12. The popped `DiscardAll` corresponds to a `CommandComplete` whose tag is `DISCARD ALL`, so that the tag-driven `complete_deallocation` pass (be L163-L170) and this command-driven pass act on the same event.
    Established by: nothing found; the two passes are selected independently (be L166 by tag, sp L294-L297 by queue head).

13. The caller holds the `protocol_state` write lock for the duration of the call and the dependent prunes.
    Established by: ctx L1576-L1601.

## 9. Shared state

Read and written, all fields of `SettingsProtocol` inside `ConnectionProtocolState` (ctx L156), behind the `protocol_state` lock shared by the Frontend and Backend halves of one connection:

- `pending` (sp L132): read L288, mutated L289, entry removed L292.
- `settings_since_boundary` (sp L134): raised L290.
- `prepared` (sp L129): pruned L299-L300.
- `portals` (sp L130): pruned L301.
- `journal` (sp L131): pruned L302, values rewritten L303-L322.

Not touched: `boundaries` (sp L133), `discarding_until_sync` (sp L135).

Written by the caller as a direct consequence of the return value, under the same lock: `ConnectionProtocolState.settings` (ctx L1582), `prepared_statements`, `portals`, `portal_operations` (ctx L1594-L1596), `statement_metrics` (ctx L1598).

No process-wide state is reached from this function.

## 10. Open questions

- Unclear whether sqltk's `Parser::parse_statements` emits a `Statement` for an empty statement (for example `;;`) or for a trailing comment; need to inspect the sqltk parser used at `packages/cipherstash-proxy/src/postgresql/parser.rs` L72-L85 to know whether an `EmptyQueryResponse` position has a queued command.
- Unclear whether pg-proto attaches `Some(operation)` to every `CommandComplete`, and which id it attaches for completions that follow `CopyDone` or a multi-statement Query; need to inspect pg-proto 0.12.0 operation correlation.
- Unclear whether pg-proto's `OperationId` ordering is strictly monotonic in frontend message order across the lifetime of a connection (including wrap or reuse); need to inspect pg-proto 0.12.0 `OperationId`.
- Unclear whether the `CommandComplete` payload from pg-proto includes the trailing NUL; `is_deallocation` strips an optional `\0` (be L764) while `command_succeeded` compares the lossy string with `"ROLLBACK"` exactly (be L98, st L325); need to inspect pg-proto's `BackendMessage::CommandComplete` decoding.
- Unclear which journal entries can still be keyed at or below a `DISCARD ALL` operation when its `CommandComplete` arrives (they are normally removed by `prepared_or_bound` on ParseComplete/BindComplete/CloseComplete, sp L326-L330); need to inspect the locally refused Parse/Bind/Close paths in `frontend.rs` for mutations that never receive an acknowledgement.
- Unclear whether a Context portal removed at ctx L1595 while a later pipelined operation still depends on it is read from the Context table or from the `ExecuteContext` clone (ctx L669-L670, L682-L696) during row handling; need to inspect `Backend::data_row_handler` and `Context::get_portal` call sites in `backend.rs`.
- Unclear whether `suspended_executions` entries for portals removed by the discard are left in place until readiness (ctx L1587-L1598 does not prune them; ctx L967-L981 does at Idle); need to inspect readers of `suspended_executions` between the two points.
- Unclear whether a Context `prepared_statements` entry can exist with no `SettingsProtocol.prepared` entry of the same name outside the Parse handler window (lc L109-L159); need to inspect `Frontend::parse_handler` early-return paths and `PreparedStatements::set_statement`/`set_deallocation` callers (ps L57-L83), which create entries with `or_default`.
