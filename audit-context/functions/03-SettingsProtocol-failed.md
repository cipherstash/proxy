## `SettingsProtocol::failed` in packages/cipherstash-proxy/src/postgresql/context/settings_protocol.rs (L331-L432)

File references without a path are to `settings_protocol.rs`.
Other files are abbreviated: `context/mod.rs`, `context/lifecycle.rs`, `context/settings.rs`, `middleware/frontend.rs`, `middleware/backend.rs`, `middleware/mod.rs`, `postgresql/mod.rs`, all under `packages/cipherstash-proxy/src/postgresql/`.

**Purpose:**
Backend-side rollback of the speculative settings journal when PostgreSQL reports an `ErrorResponse` correlated to a frontend operation.
The Frontend applies Parse/Bind/Close effects to `prepared` and `portals` before PostgreSQL answers (L160-L185, L191-L231, L524-L544) and records an undo entry per operation in `journal` (L131).
`failed` decides which later operations PostgreSQL skipped, undoes their journal entries, drops their pending settings commands, and sets `discarding_until_sync` when the Frontend has not yet seen the message that ends the skip.
It returns the Context-level metadata snapshots the caller must reinstall and the metrics scope ids whose journal reference was dropped (L331-L336).
Without it, `prepared`/`portals` (and, through the caller, `prepared_statements`/`portals`/`portal_operations`/`suspended_executions` in `ConnectionProtocolState`) keep objects PostgreSQL never created, and the Frontend keeps forwarding messages PostgreSQL is ignoring.

---

**Inputs & Assumptions:**

- `operation: Option<OperationId>`: the frontend operation pg-proto correlated with the `ErrorResponse`. Trust: produced by pg-proto (trusted, source not on disk); the *timing and existence* of the `ErrorResponse` is driven by upstream PostgreSQL (semi-trusted) reacting to client-shaped messages (semi-trusted). It arrives via `middleware/mod.rs` L65-L76 -> `Backend::intercept` (`middleware/backend.rs` L148-L154) -> `report_outcome` L109 -> `postgres_failed` (`context/lifecycle.rs` L473-L478) -> `settings_execution_failed` (`context/mod.rs` L1608-L1613) -> `restore_failed_preparation` (`context/mod.rs` L220).
- Implicit state read: `boundaries` (L340-L344), `pending` (L350-L364), `journal` (L365-L371, L380, L395-L402), `prepared` / `portals` (L409-L421).
- Implicit: `OperationId` total order. It is `#[derive(Ord)]` over a wrapper of `pg_proto::OperationId` (`postgresql/mod.rs` L20-L28); the ordering itself is defined inside pg-proto.
- Implicit: exclusive access. The function takes `&mut self`; the only caller holds the `protocol_state` write lock (`context/mod.rs` L1609-L1613).

Preconditions, and what establishes each:

1. `OperationId` order equals frontend message arrival order on this connection, with no reuse. Established by: nothing found in this repository; `postgresql/mod.rs` L20-L34 only forwards pg-proto's `Ord`. pg-proto is a black box.
2. The `OperationId` passed is the id of the frontend message whose processing PostgreSQL failed. Established by: nothing found in this repository; `middleware/mod.rs` L70-L75 forwards pg-proto's value unchanged.
3. `boundaries` is in ascending id order, so the first element `>= operation` is the nearest boundary. Established by: `boundary` appends (L436-L438) and is called only from Frontend-side paths (`simple_query` L284, `sync` L441, `readiness_boundary` at `context/lifecycle.rs` L87), each with the current frontend operation id; ascending order then follows from precondition 1. Nothing sorts or checks the Vec.
4. Every frontend message that ends PostgreSQL's error-skip has pushed a boundary before `failed` looks for it, or has not yet been seen by the Frontend at all. Established by: Sync at `middleware/frontend.rs` L182, L207, L328 (all reach `sync`, L439-L441, through `context/mod.rs` L1559-L1566); simple Query at `context/lifecycle.rs` L62 (config-disabled mapping), `register_query` -> `simple_query` L284, or `refuse_query` -> `settings_protocol_boundary` (`context/lifecycle.rs` L394); FunctionCall at `context/lifecycle.rs` L75-L77.
5. Backend responses are delivered in frontend-operation order, so no operation later than `operation` has been acknowledged (`prepared_or_bound`, L326-L330) or retired (`ready`, L459-L468) before this call. Established by: nothing found in this repository; the same assumption is written as a comment at L493 for `deallocated`.
6. A journal entry at key `operation` whose name is empty and whose `close` is false means PostgreSQL destroyed the previous unnamed statement/portal before failing. Established by: nothing found; the rule is asserted in the comment at L381-L382 and applied at L383.

---

**Outputs & Effects:**

- Returns `(Vec<RestoreMetadata>, Vec<SessionId>)` (L431). `restored` is ordered by descending operation id (L370, L425). `released` is unfiltered candidate ids; the caller filters them with `take_unreferenced_metrics` (`context/mod.rs` L265, L324-L335).
- `operation == None`: returns two empty Vecs and writes nothing (L337-L339).
- Writes `discarding_until_sync` (OR-assign, L345).
- Writes `settings_since_boundary` (OR-assign, L350-L353).
- Removes or empties `pending` entries in the skipped range (L354-L364).
- Removes every `journal` entry in the skipped range (L380) and rewrites `previous` and `metadata` of at most one later entry per removed entry (L403-L404).
- Removes and conditionally reinserts entries in `prepared` (L409-L413) and `portals` (L417-L421).
- No external calls, no awaits, no logging.

Postconditions on return with `operation == Some(op)`:

- No `journal` key `k` with `k >= op` and (`boundary` absent or `k <= boundary`) remains (L365-L371, L380).
- No `pending` key in that range remains unless it had `until_ready && barrier`, and those have empty `commands` (L354-L364).
- `discarding_until_sync` is true if no recorded boundary was `>= op` (L345). It is never cleared here; `sync` (L440) and `ready` (L457) clear it.

---

**Block-by-Block:**

```rust
// L337-L339
let Some(operation) = operation else {
    return (Vec::new(), Vec::new());
};
```
- **What:** An uncorrelated failure is a no-op for this structure.
- **Why here:** Every later step is expressed relative to an id.
- **Assumes:** An `ErrorResponse` with no operation invalidates no speculative Parse/Bind/Close and starts no skip. Established by: nothing found.
- **Establishes:** Nothing. Note the caller still runs `self.settings.execution_failed()` (`context/mod.rs` L264) on this path, so `ConnectionSettings.failed` is set while the journal, `pending`, and `discarding_until_sync` are unchanged.
- **Depended on by:** `restore_failed_preparation` iterates an empty `restored` (`context/mod.rs` L221).

```rust
// L340-L345
let boundary = self.boundaries.iter().copied().find(|boundary| *boundary >= operation);
self.discarding_until_sync |= boundary.is_none();
```
- **What:** Picks the first recorded boundary (Vec insertion order) not earlier than the failed operation; when none exists, marks the connection as discarding.
- **Why here:** The boundary fixes the upper end of the skipped range used by every later block.
- **Assumes:**
  - Insertion order equals id order (precondition 3). `find` returns the first match in Vec order, not the minimum.
  - `boundaries` holds one undifferentiated list. `boundary()` (L436-L438) is reached for Sync (L441), simple Query (L284, `context/lifecycle.rs` L62), and FunctionCall (`context/lifecycle.rs` L75-L77, L87). `failed` treats any of them as the end of the skipped range. Whether a simple Query or FunctionCall that follows a failed extended-protocol message (with no Sync between) ends PostgreSQL's skip is not established anywhere in the code; `context/lifecycle.rs` L72-L74 only states that PostgreSQL never discards later messages after a FunctionCall's *own* failure.
  - A boundary `>= operation` has not been pruned. `ready` removes boundaries `<= ` its operation (L460); under precondition 5 the `ReadyForQuery` for a boundary arrives after every failure it bounds.
  - `ready(None, _)` never prunes `boundaries` (L458-L460), so stale entries persist until a correlated readiness; they are all `<` any later operation and do not match the `>=` test.
- **Establishes:** `boundary.is_none()` is the only condition under which this function sets `discarding_until_sync`.
- **Depended on by:** `Frontend::intercept` reads the flag through `discarding_extended_messages` (`context/mod.rs` L1550-L1557) at `middleware/frontend.rs` L179 and suppresses every non-Sync, non-Terminate message (L185-L186), including simple Query and FunctionCall, without recording a boundary or journal entry for them.
- **Observed mismatch:** The test comment at L811-L812 says a FunctionCall or configuration-disabled Query "records no boundary"; `context/lifecycle.rs` L58-L63 and L75-L77 do record one through `readiness_boundary` (L81-L89). The test (L809-L817) exercises the no-boundary case directly and relies on `ready` clearing the flag (L457).

```rust
// L346-L347
let skipped = |id: &OperationId| *id >= operation && boundary.is_none_or(|boundary| *id <= boundary);
```
- **What:** Predicate for "PostgreSQL did not apply this operation": the closed range `[operation, boundary]`, or `[operation, +inf)` when no boundary is recorded.
- **Assumes:** Operations in `(boundary, +inf)` belong to a later batch that PostgreSQL processes normally. The range is inclusive at both ends: the failed operation itself and the boundary operation itself are in it.
- **Depended on by:** L350-L353, L354-L364, L365-L371.

```rust
// L350-L353
self.settings_since_boundary |= self.pending.iter().any(|(id, pending)| skipped(id) && pending.barrier);
```
- **What:** If any skipped pending entry is a barrier, records that settings may change at the next readiness.
- **Why here:** It reads `pending` before L354 removes the entries it inspects.
- **Assumes:** The only failures that change effective settings at `ReadyForQuery` are those where a pending entry in the skipped range has `barrier == true`. `barrier` is computed at Execute/Query time by `SettingsCommand::may_change_effective` (`context/settings.rs` L189-L209) against the then-confirmed settings (L251, L269-L275). A failed operation with no pending entry, or with `barrier == false`, does not set the flag here; `ConnectionSettings::ready_for_query` restores `transaction_start` on Idle whenever `failed` is set (`context/settings.rs` L372-L378), independently of this flag.
- **Establishes:** With the flag set, the next `sync` inserts a barrier `Pending` (L442-L450), so `in_flight()` (L521-L523) stays true until `ready` (L455, L459).
- **Depended on by:** `sync` L442; `settings_in_flight` (`context/mod.rs` L1621-L1628), consulted by the driver at `driver.rs` L220 and L258-L259.
- **Note:** The predicate matches barriers that were skipped (never ran) as well as the one that failed; the comment at L348-L349 names only the failed case.

```rust
// L354-L364
self.pending.retain(|id, pending| {
    if !skipped(id) { return true; }
    if pending.until_ready && pending.barrier { pending.commands.clear(); true } else { false }
});
```
- **What:** Drops skipped pending entries. Entries with `until_ready && barrier` survive with their command queue emptied.
- **Assumes:** `until_ready` entries are created only by `simple_query` (L281) and `sync` (L448). No further `CommandComplete` for a skipped operation arrives; if one did, `succeeded` returns `None` through L288 (entry gone) or L289 (queue empty) and applies no setting.
- **Establishes:** A failed barrier simple Query, and the Sync barrier of a failed batch, keep `in_flight()` true until `ready` removes them (L459). Test at L942-L957.
- **Depended on by:** `succeeded` L288-L293; `in_flight` L521-L523; `sync` L442.

```rust
// L365-L371
let mutations: Vec<_> = self.journal.keys().filter(|id| skipped(id)).copied().rev().collect();
```
- **What:** Collects skipped journal keys newest-first.
- **Why here:** Undoing in descending order means each undo sees the live state its own forward mutation left behind, and makes the rebase search at L395 see only entries beyond the skipped range.
- **Establishes:** When iteration reaches `id`, every skipped key `> id` has already been removed (L380).
- **Depended on by:** L395-L402 (rebase target lies beyond the boundary), ordering of `restored`.

```rust
// L375-L392
let Mutation { name, mut previous, mut metadata, close } = self.journal.remove(&id).unwrap();
let retain_previous = !name.is_empty() || id != operation || close;
let statement = matches!(previous, Previous::Statement(_));
if !retain_previous {
    previous = if statement { Previous::Statement(None) } else { Previous::Portal(None) };
    released.extend(std::mem::take(&mut metadata).metrics());
}
```
- **What:** Takes the undo record. For exactly one shape — unnamed, the failed operation itself, not a Close — it discards the saved previous object and the saved Context snapshot.
- **`unwrap`:** The key came from `self.journal.keys()` at L365-L371, and between collection and this line the loop only removes keys from that same list (L380), each once; the rebase path mutates values, not keys (L403-L404). The function holds `&mut self` throughout.
- **Assumes (unnamed rule):** An `ErrorResponse` correlated with an unnamed Parse or Bind implies PostgreSQL had already destroyed the prior unnamed statement or portal. The code keys this on `id == operation` and name emptiness only (L383); it does not consult the error content or the point at which PostgreSQL failed. Established by: nothing found (comment L381-L382).
- **Assumes (journal presence):** A journal entry exists at `operation` only when the failed message was a Parse, Bind, or Close handled in mapped mode. `start_prepare` L145, `prepare` L178, `bind` L201, `close` L535 are the only inserters. With config-disabled mapping, `unmapped_parse` and `unmapped_bind` write no journal entry (`context/lifecycle.rs` L42-L56) and `close` skips it (`context/lifecycle.rs` L323). In the Frontend discard path nothing is journaled (`middleware/frontend.rs` L185).
- **Refused-locally case:** When the Frontend refuses a Parse or Bind it forwards a substitute designed to fail upstream (`middleware/frontend.rs` L270-L272 with L1731-L1743; L295-L297 with L1745-L1754). The journal entry for that operation already exists (`start_prepare` via `context/lifecycle.rs` L99-L102; `bind` L201-L209 runs before the refusal return at L218). The unnamed rule is then applied to PostgreSQL's response to the substitute. The substitute Bind names a nonexistent statement (`middleware/frontend.rs` L1752); whether PostgreSQL destroys the unnamed portal before rejecting that Bind is not established in the code.
- **Establishes:** `metadata` is `MetadataSnapshot::default()` whenever `retain_previous` is false, so a `RestoreMetadata` pushed for it carries nothing to reinstall.
- **Depended on by:** L403-L404, L410, L418, and the caller's restore loop (`context/mod.rs` L221-L263).

```rust
// L395-L406
if let Some((_, later)) = self.journal
    .range_mut((Excluded(id), Unbounded))
    .find(|(_, mutation)| mutation.name == name && matches!(mutation.previous, Previous::Statement(_)) == statement)
{
    later.previous = previous;
    released.extend(std::mem::replace(&mut later.metadata, metadata).metrics());
    continue;
}
```
- **What:** If a later, still-journaled operation touched the same name and kind, the undo record is transplanted onto that entry and the live maps are left alone.
- **Why here:** A later pipelined batch has already overwritten the live object; restoring over it would clobber an operation PostgreSQL is going to apply (comment L393-L394). Tests at L576-L594 and L596-L643.
- **Assumes:**
  - The first later entry with matching name and kind is the one whose `previous` was produced by the mutation being undone. The match is on `name` and kind only (L399-L400); it does not compare `Prepared.operation` / `Bound.operation` of `later.previous` with `id`.
  - Because of the descending order (L370), every candidate has an id above the boundary; when `boundary` is `None`, the range `(id, +inf)` is already empty of journal entries and this block never matches.
  - `later.previous` and `later.metadata` are overwritten unconditionally, including when another function previously expired them to `None`/default: `succeeded` L316-L320 (DISCARD ALL), `ready` L472-L476 (Idle), `deallocated` L507-L508. Under precondition 5 those expirations are triggered by responses that arrive after this failure.
- **Establishes:** The later entry now undoes to the state before the failed operation. Neither `prepared`/`portals` nor a `RestoreMetadata` is produced for this `id` (`continue`, L405).
- **Depended on by:** A later `failed` or `ready` on the rebased entry.
- **Scope of the rebase:** It covers the journal entry for the same name and kind. Objects in a later batch that were *derived* from the rolled-back object are not visited: `bind` stores a clone of the `Prepared` inside `Bound` (L74, L210, L225), and `begin_bind` reads the Context statement for the Bind at `context/lifecycle.rs` L249. A later-batch portal bound from a statement that this call rolls back keeps that clone. Nothing in this function, and nothing found in its caller (`context/mod.rs` L216-L266), revisits those portals.

```rust
// L407-L424
match previous {
    Previous::Statement(previous) => {
        self.prepared.remove(&name);
        if retain_previous { if let Some(previous) = previous { self.prepared.insert(name.clone(), previous); } }
    }
    Previous::Portal(previous) => { /* same over self.portals */ }
}
```
- **What:** Removes whatever is live under the name, then reinstalls the saved object when the rule at L383 allows it and one was saved.
- **Assumes:** The live entry under `name` is the product of operation `id` or of nothing. The removal at L409 / L417 is unconditional; it does not check `prepared[name].operation == id`. This holds when no later operation on the same name exists outside the journal, which reduces to precondition 5 plus the rebase block having caught every later journaled entry.
- **Establishes:** `prepared[name]` / `portals[name]` equals the pre-`id` state, or is absent under the unnamed rule.
- **Redundancy:** After L385-L390 `previous` is already `None` when `retain_previous` is false, so the `if retain_previous` guards at L410 and L418 never change the outcome.
- **Depended on by:** `bind` L210, `execute` L239, `has_statement` / `has_portal` L515-L520 (used by `settings_execution_succeeded`, `context/mod.rs` L1594-L1596).

```rust
// L425-L431
restored.push(RestoreMetadata { name, statement, metadata });
...
(restored, released)
```
- **What:** Emits one Context-restore instruction per non-rebased undo.
- **Assumes:** The caller applies `restored` in the returned order. With two skipped entries on one name, the newer is emitted first and the older last, so the older snapshot is the one left installed (`context/mod.rs` L221-L263 removes then reinstalls per entry).
- **Depended on by:** `restore_failed_preparation`.

---

**Cross-Function Dependencies:**

Callees:

- `MetadataSnapshot::metrics` (internal, L98-L107): yields `statement_metrics`, the portal's `session_id()`, and the suspended execute's `session_id()`. `failed` depends on it to enumerate every metrics scope a dropped snapshot referenced (L391, L404). It has one path and no early return. It does not yield anything for `statement`, `statement_operation`, `statement_deallocation`, or `portal_operation`; those fields carry no `SessionId` at L89-L95.
- `BTreeMap::range_mut` / `keys` / `remove`, `Vec::iter().find`, `Option::is_none_or` (std): ordering semantics come from `Ord for OperationId`.
- `Ord for OperationId` (external-black-box through `pg_proto::OperationId`, `postgresql/mod.rs` L20-L28): `failed` depends on it for L344, L347, L370, L397. Not excluded: ids that are not monotonic in frontend arrival order, id reuse or wrap, a correlated id that never passed through `Frontend::intercept`.

Callers:

- `ConnectionProtocolState::restore_failed_preparation` (`context/mod.rs` L216-L266) is the only non-test caller. For each `RestoreMetadata` it removes the live Context statement or portal/portal owner/suspended execution under the name, reinstalls whichever snapshot fields are `Some` (L222-L262), then calls `self.settings.execution_failed()` (L264) and filters metrics with `take_unreferenced_metrics` (L265). It assumes `failed` returns every name whose Context metadata needs restoring; names handled by the rebase path are not returned and their live Context metadata is left as the later operation wrote it.
- That caller is reached from `settings_execution_failed` (`context/mod.rs` L1608-L1619) <- `postgres_failed` (`context/lifecycle.rs` L473-L488) <- `Backend::report_outcome` on `ErrorResponse` (`middleware/backend.rs` L109). `report_outcome` runs on the passthrough path (L188), the `discard_execution` path (L219), the flush-failure path when the message is correlated (L244-L246), and the normal path (L254).
- Not a caller: `local_execution_failed` (`context/lifecycle.rs` L490-L498). A Proxy-generated decryption error (`middleware/backend.rs` L122-L128, L236-L247, L296-L299) does not reach `failed`; later `ParseComplete`/`BindComplete` in the same batch still retire journal entries at `middleware/backend.rs` L172-L179, ahead of the `discard_execution` branch at L218.

Shared state (all fields of `SettingsProtocol`, L128-L136):

- `boundaries`: written by `boundary` L437, `ready` L460.
- `discarding_until_sync`: written by `sync` L440, `ready` L457; read by `discarding` L433-L435.
- `settings_since_boundary`: written by `succeeded` L290, `ready` L455; read by `sync` L442.
- `pending`: written by `execute` L252, `simple_query` L276, `sync` L443, `succeeded` L288-L293, `ready` L459.
- `journal`: written by `start_prepare` L145, `prepare` L178, `bind` L201, `close` L535, `prepared_or_bound` L327-L329, `succeeded` L302-L322, `ready` L461-L478, `deallocated` L496-L509; read by `references_metrics` L155-L159.
- `prepared` / `portals`: written by `prepare` L168, `capture_literals` L187-L189, `bind` L200, L221, `close` L532-L533, `succeeded` L299-L301, `ready` L470, `deallocated` L486-L492.

Invariant couplings:

- Serialisation. Frontend and Backend middleware are fields of one struct invoked through `&mut self` (`middleware/mod.rs` L54-L76), and every access to `SettingsProtocol` goes through the `protocol_state` lock. The interleaving that matters is logical: how many frontend messages `Frontend::intercept` has processed when the `ErrorResponse` for `operation` reaches the Backend. That is decided by pg-proto and the network. `failed` has two regimes keyed on it: boundary already recorded (range closed, no discard flag) and boundary not yet recorded (range open, discard flag set, Frontend suppresses until Sync at `middleware/frontend.rs` L179-L187).
- `sync` evaluates `settings_since_boundary || in_flight()` at the moment the Frontend processes Sync (L442). When the Sync precedes the failure, the Sync's barrier `Pending` exists only if a barrier pending existed then; `failed` preserves it through the `until_ready && barrier` branch (L358-L360). When the failure precedes the Sync, L350-L353 carries the fact forward. `ready` resets `settings_since_boundary` on every `ReadyForQuery`, correlated or not (L455).
- `ready` (L459-L468) retires journal and pending entries `<= ` its operation with no rollback. `failed` for an operation at or below an already-ready boundary would find nothing to undo. Ordering between the two rests on precondition 5.
- `ConnectionSettings.failed` is set by the caller for every `ErrorResponse` (`context/mod.rs` L264), including the `None` path that `failed` ignores.

---

**Open Questions:**

- unclear; need to inspect pg-proto 0.12.0 `OperationId` (allocation, `Ord`, wrap behaviour) to confirm ids are strictly increasing in frontend arrival order.
- unclear; need to inspect pg-proto's backend correlation to learn which `OperationId` it attaches to an `ErrorResponse` raised during COPY sub-protocol messages (CopyData/CopyDone/CopyFail fall into the `_` arm at `middleware/frontend.rs` L336-L342 and record no boundary), and whether it attaches `Some` or `None` to asynchronous errors.
- unclear; need to inspect pg-proto's ordering guarantee between `frontend_operation` and `backend_operation` invocations, and what it does when `Frontend::intercept` returns `Err` after journaling (for example `Bind::try_from(bind)?` at `middleware/frontend.rs` L279 or `describe_handler(...)?` at L237) — whether the message is forwarded, dropped, or the connection closed.
- unclear; need to inspect PostgreSQL's handling of a simple Query or FunctionCall received while it is skipping to Sync, against L340-L344 treating their boundaries as the end of the skipped range.
- unclear; need to inspect PostgreSQL's Parse and Bind failure points (before or after dropping the unnamed statement/portal) against the rule at L383, including the substitute Bind built at `middleware/frontend.rs` L1745-L1754.
- unclear; need to inspect `driver.rs` L140-L270 to determine whether anything holds a later pipelined Bind/Execute until an earlier Parse on the same statement name is acknowledged; the gate visible at L220 and L258-L259 consults `settings_in_flight` and `schema_ddl_in_flight`, not the journal.
- unclear; need to inspect `Frontend::bind_handler` / `parse_handler` under session-level `unsafe_disable_mapping` to confirm journal entries are still written on that path.
- unclear; need to inspect `PreparedStatements::remove` / `set_*` (`context/` module, not read) to confirm the caller's remove-then-reinstall at `context/mod.rs` L223-L239 fully replaces the per-name record.
