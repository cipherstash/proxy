# ConnectionSettings::command_succeeded

File: `packages/cipherstash-proxy/src/postgresql/context/settings.rs`, L317-L366.
Signature: `pub fn command_succeeded(&mut self, command: &SettingsCommand, command_tag: &str)`.
All `settings.rs` line references below are to that file unless another file is named.

Note on format: `ANALYSIS_FORMAT.md`, `FUNCTION_MICRO_ANALYSIS_EXAMPLE.md` and `DOMAIN_NOTES.md` were not found on disk (glob for `**/audit-context-building/resources/*.md` returned nothing), so this document follows the section order implied by the task brief.

## 1. Purpose

Applies the settings effect of one statement to the per-connection `ConnectionSettings` (L291-L299).
It is the only function that changes `keyset` and `unsafe_disable_mapping` in response to client SQL: the other writers (`ready_for_query` L372-L379, `restore_transaction` L381-L386, `end_transaction` L407-L413, `reset` L416-L418) only restore, clear overrides, or default.
A repo-wide search for `settings.reset(`, `settings.set_keyset(`, `settings.set_unsafe_disable_mapping(`, `settings.end_transaction(` and `.settings =` outside `settings.rs` returned no production call sites, so the public setters at L393 and L402 are reached only through L352 and L354.

The function has no return value and no error path.
It cannot refuse a command: every input either mutates state or is silently a no-op.

## 2. Inputs and who controls them

| Input | Source | Controlled by |
| --- | --- | --- |
| `self` | `ConnectionProtocolState.settings` (`context/mod.rs:155`, default at `:175`), or a clone of it in projection | Prior calls on this connection |
| `command` | Parsed on the frontend path by `SettingsCommand::from_statement` (L170-L187) and queued in `SettingsProtocol.pending`; popped by `SettingsProtocol::succeeded` (`settings_protocol.rs:287-289`) | Authenticated client SQL, through the sqltk parser |
| `command_tag` | Production: the `CommandComplete` payload, `String::from_utf8_lossy(tag)` at `middleware/backend.rs:98`, passed through `ExecutionEnd::Command` (`lifecycle.rs:453-454`) and `mod.rs:1582`. Projection: `success_tag()` (L211-L217) or the literal `"ROLLBACK"` (L205) | Production: upstream PostgreSQL and pg-proto framing. Projection: Proxy |

`command_tag` is read at exactly one place, L325.
On every other branch the tag is ignored and the call itself is the only evidence of success.

## 3. Callers

1. `Context::settings_execution_succeeded`, `context/mod.rs:1568-1606`, at `:1582`.
   This is the only production caller that mutates the live settings.
   It runs under the `protocol_state` write lock (`mod.rs:1576-1579`).
   It returns `Ok(())` without calling when `operation` is `None` (`mod.rs:1573-1575`) and when `SettingsProtocol::succeeded` returns `None` (`mod.rs:1581`).
   It is reached from `Context::execution_completed` only for `ExecutionEnd::Command` (`lifecycle.rs:453-455`); `ExecutionEnd::Empty` and `ExecutionEnd::Suspended` do not reach it.
   `execution_completed` calls it before looking at `discarded` (`lifecycle.rs:453` precedes `:458`), so it runs for discarded replies too.
   `Backend::report_outcome` (`backend.rs:89-112`) is invoked on the passthrough path (`backend.rs:188`), the discard path (`backend.rs:219`), the flush-failure path (`backend.rs:245`) and the normal path (`backend.rs:254`).
2. `SettingsCommand::may_change_effective`, L198 and L205, on clones (`settings.clone()` at L197 and L204).
3. `project_settings_effects`, `settings_protocol.rs:57`, on a clone (`settings_protocol.rs:51`).
4. `ConnectionSettings::execution_succeeded`, L310-L315, `#[cfg(test)]` only.

Callers 2 and 3 are speculative: they run before PostgreSQL has seen the statement and pass a synthetic tag.
The doc comment at L308 ("Applies only a command confirmed successful by PostgreSQL") is attached to the test-only wrapper at L310, not to `command_succeeded`, and does not hold for callers 2 and 3.

## 4. Block-by-block

### 4.1 Lazy transaction baseline, L318-L322

`self.transaction_start.get_or_insert_with(...)` stores a `SettingsSnapshot` of the current `keyset` and `unsafe_disable_mapping` when `transaction_start` is `None`.
It runs for every `command` variant, including `Other` and `Begin` (the `match` starts at L323, after it).
It runs before any mutation in this call, so the snapshot holds the pre-command values.
Both `Scoped` fields are cloned whole, so the snapshot includes the `transaction` override (L247) as well as the `session` value (L245).

`transaction_start` is cleared at three places: `restore_transaction` via `take()` (L382), `end_transaction` (L410), and `reset` (L417, whole-struct default).

Consequence that the rest of the function relies on: at L326 `transaction_start` is always `Some`, so `restore_transaction` at L326 always assigns both fields.
When the Commit/Rollback command is itself the first command since the last clear, the snapshot was taken at L318 of the same call and the restore is a no-op.

The snapshot is taken at the first *applied* command, not at the PostgreSQL transaction start.
Statements for which caller 1 does not call (`operation == None`, or no `Pending` entry: `SettingsProtocol::execute` returns `Ok(false)` without inserting when the portal is unknown, `settings_protocol.rs:239-241`) do not trigger it.
That late capture yields the same values as an early one because every mutation of `keyset`/`unsafe_disable_mapping` between two clears goes through this function, after L318.

### 4.2 `Commit | Rollback`, L324-L329

```
if command_tag == "ROLLBACK" { self.restore_transaction(); }
self.end_transaction();
```

The branch is shared by both variants; the variant is not consulted again.
The commit/rollback decision is made only by byte-exact `&str` equality with `"ROLLBACK"` at L325.
`self.failed` is not read here.
`SettingsCommand::Rollback` with any tag other than exactly `"ROLLBACK"` takes the commit path: no restore, then `end_transaction`.
`SettingsCommand::Commit` with tag `"ROLLBACK"` takes the rollback path; L202 and the test at L687 document this as the aborted-transaction COMMIT case.

`end_transaction` (L407-L413) then clears both `transaction` overrides (L408-L409, via `Scoped::end_transaction` L284-L286), sets `transaction_start = None` (L410), `failed = false` (L411) and clears `savepoints` (L412).
After this branch there is nothing left for a later `ready_for_query(Idle)` (L372-L379) to restore: `failed` is false and `transaction_start` is `None`.
The decision made at L325 is therefore final for that transaction.

The commit path keeps `session` values written inside the transaction and drops `transaction` overrides.
The rollback path replaces both `Scoped` values with the baseline, then drops the baseline's own `transaction` override at L408-L409.

This branch is the only transaction end the function sees for `COMMIT AND CHAIN` / `ROLLBACK AND CHAIN`; the test at L695-L736 exercises that with no intervening `ready_for_query(Idle)`.

### 4.3 `Savepoint(name)`, L330-L336

Pushes `(name.clone(), snapshot of current keyset + unsafe_disable_mapping)` onto `self.savepoints`.
No check for an existing entry with the same name; duplicates are kept (test L739-L779 depends on that).
No check that a transaction is open.
No upper bound on `savepoints.len()`; the vector is emptied only by `end_transaction` (L412), `reset` (L417), and shortened by L342 and L348.

### 4.4 `RollbackTo(name)`, L337-L345

`rposition` finds the last entry whose stored name equals `name` by `String` equality (L338).
Found: both fields are replaced from the entry's snapshot (L339-L341), entries after it are dropped and the entry itself is kept (`truncate(index + 1)`, L342), and `failed` is set to `false` (L343).
Not found: no state changes at all. `failed` is not cleared, the settings are not restored, and nothing is reported.

`transaction_start` is not touched on either path, so a later full rollback still restores the transaction baseline.

### 4.5 `Release(name)`, L346-L350

Found: `truncate(index)` drops the entry and everything after it (L348); settings are unchanged.
Not found: no-op.

### 4.6 `Change(change)`, L351-L363

- `Keyset(scope, keyset)` → `set_keyset` (L352, L393-L395) → `Scoped::assign` (L268-L278).
- `Mapping(scope, disabled)` → `set_unsafe_disable_mapping` (L353-L355, L402-L404) → `Scoped::assign`.
- `ResetKeyset(scope)` / `ResetMapping(scope)` → `Scoped::reset` (L361-L362, L264-L266) → `assign(scope, None)`.
- `ResetAll` → both fields become `Scoped::default()` (L357-L360). `transaction_start`, `failed`, `savepoints` are untouched, so the reset is covered by rollback (tests L585-L637).
- `DiscardAll` → `self.reset()` (L356, L416-L418) → `*self = Self::default()`. This also sets `transaction_start = None`, `failed = false` and empties `savepoints`, discarding the baseline captured at L318 in this same call.

`Scoped::assign` semantics the caller depends on:
`Session` writes `session = value` and clears `transaction` (L273-L274);
`Transaction` writes `transaction = Some(value)` (L276), where `Some(None)` is an explicit local default that hides the session value (comment L246; `current()` at L280-L282).

No `Change` arm reads `failed` or `command_tag`.
A `Change` is applied whether or not `failed` is already `true`.

### 4.7 `Other | Begin`, L364

No effect beyond the baseline capture at L318.
`Begin` does not reset `transaction_start` or `savepoints`.

## 5. Callees

All callees are internal to `settings.rs` and infallible.

| Callee | Lines | What the caller depends on |
| --- | --- | --- |
| `Option::get_or_insert_with` (std) | L318-L322 | Stores only when `None`; never overwrites an existing baseline |
| `restore_transaction` | L381-L386 | `take()`s the baseline and assigns both fields. Does nothing when the baseline is `None`; unreachable from L326 because of L318, reachable from `ready_for_query` L375 |
| `end_transaction` | L407-L413 | Clears both `transaction` overrides, baseline, `failed`, `savepoints`. Does not touch `session` values |
| `set_keyset` / `set_unsafe_disable_mapping` | L393-L395, L402-L404 | Thin wrappers over `Scoped::set` |
| `Scoped::set` / `reset` / `assign` | L260-L278 | Session assignment clears the transaction override; transaction assignment leaves session untouched |
| `reset` | L416-L418 | Replaces the whole struct with `Default` |
| `Vec::iter().rposition`, `truncate`, `push` (std) | L330, L338, L342, L347, L348 | Last-match lookup; `truncate(index + 1)` keeps the matched entry, `truncate(index)` removes it |

There are no external calls, no I/O, no `await`, no locking inside the function.
It cannot be re-entered: it holds `&mut self` and the production caller holds the `RwLock` write guard (`mod.rs:1576-1579`).

Upstream functions that establish this function's preconditions (read because the function's correctness rests on them):

- `SettingsProtocol::succeeded` (`settings_protocol.rs:287-325`): returns `None` when no `Pending` exists for `operation` (L288) and when the command queue is empty (`pop_front` at L289 returns `None`, returned at L324). Otherwise it returns the front command. Pairing of command to `CommandComplete` is positional per operation.
- `SettingsProtocol::simple_query` (`settings_protocol.rs:262-286`): queues one command per parsed statement, in parse order.
- `SettingsProtocol::execute` (`settings_protocol.rs:232-261`): queues exactly one command per Execute, cloned from the bound portal's prepared statement (L250).
- `SettingsProtocol::failed` (`settings_protocol.rs:333-364`): on an `ErrorResponse`, removes skipped pending entries or clears their queues (L354-L364), which is what stops later positional pops for the failed operation.
- `SettingsCommand::from_statement` (L170-L187) and `postgres_identifier` (`proxy/schema/middleware.rs:1037-1043`): produce the savepoint name strings compared at L338 and L347. `postgres_identifier` returns the quoted value unchanged or the ASCII-lowercased unquoted value. It does not truncate.
- `Backend::report_outcome` (`backend.rs:89-112`): converts the tag bytes with `String::from_utf8_lossy` and nothing else (L98).

## 6. Invariants

I1. The baseline is captured before any mutation in the same call (L318 precedes L323).

I2. An existing baseline is never overwritten by this function (`get_or_insert_with`, L318-L319). It is removed only at L382, L410, L417.

I3. At L326 the baseline is `Some`, so the rollback path always assigns both `keyset` and `unsafe_disable_mapping` (L318, L382-L385).

I4. After the `Commit | Rollback` branch: both `transaction` overrides are `None`, `transaction_start` is `None`, `failed` is `false`, `savepoints` is empty (L328, L408-L412).

I5. `command_tag` influences state only on the `Commit | Rollback` branch (sole read at L325).

I6. A found `RollbackTo` keeps the named savepoint and removes later ones (L342); a found `Release` removes the named savepoint and later ones (L348).

I7. A found `RollbackTo` clears `failed` (L343); a `RollbackTo` whose name is not found changes nothing (the `if let` at L338 has no `else`).

I8. `ResetAll` preserves `transaction_start`, `failed` and `savepoints` (L357-L360); `DiscardAll` clears all three (L356, L417).

I9. A session-scope assignment clears the transaction override for that setting (L273-L274).

I10. `keyset` and `unsafe_disable_mapping` are always updated in the same snapshot/restore steps: every snapshot (L319-L322, L332-L335) and every restore (L340-L341, L383-L384) handles both fields.

I11. The function is total: no panic site. `self.savepoints[index]` at L339 uses an index returned by `rposition` on the same vector at L338 with no intervening mutation.

## 7. Assumptions

A1. The call means PostgreSQL confirmed this command.
Established for the live settings by `backend.rs:96-100` (only a `CommandComplete` produces `ExecutionEnd::Command`), `lifecycle.rs:453-454`, and `mod.rs:1573-1582`.
Not established for the two projection callers (L198, L205, `settings_protocol.rs:57`), which operate on clones.

A2. `command` is the statement that produced this `CommandComplete`.
Established only positionally: `settings_protocol.rs:289` pops the front of a queue built in sqltk parse order (`settings_protocol.rs:269-275`).
Nothing compares the tag to the command kind.
A `Commit` paired with a tag of `SET`, `SELECT 1` or any other non-`ROLLBACK` string takes the commit path at L325-L328.
The pairing rests on PostgreSQL emitting exactly one `CommandComplete` per statement that sqltk parsed, in the same order, and on sqltk splitting the query string into the same statements PostgreSQL does.
Nothing found that verifies either.

A3. The tag for a rolled-back transaction is exactly the eight bytes `ROLLBACK`, with no terminator, padding, or case variation.
The value reaches L325 from `String::from_utf8_lossy(tag)` (`backend.rs:98`) with no trimming.
In the same file `is_deallocation` and `removes_all_statements` strip an optional trailing NUL from the same `CommandComplete` bytes before comparing (`backend.rs:764`, `backend.rs:770`), so the repository itself treats the presence of a trailing NUL as undetermined.
pg-proto's source is not on disk, so the byte content of `BackendMessage::CommandComplete` is not established here.
Every in-repo test constructs the tag without a NUL (for example `middleware/settings_rollback_tests.rs:123`, `middleware/frontend.rs:3642`, `middleware/mod.rs:316`).
Established by: nothing found.
If the tag carries a trailing NUL, L325 is false for every ROLLBACK and every aborted COMMIT, and L328 then discards the baseline and `failed`, so `ready_for_query` (L372-L379) cannot restore either.

A4. PostgreSQL reports a COMMIT of an aborted transaction with the tag `ROLLBACK`.
Stated in the comment at L202 and the test at L687; nothing in the code checks the transaction status that accompanies the next `ReadyForQuery` against the decision made at L325.
`self.failed` is available and is not consulted at L325.

A5. Invalid UTF-8 in the tag does not occur.
`from_utf8_lossy` (`backend.rs:98`) substitutes U+FFFD, which makes L325 false.
Established by: nothing found.

A6. Every savepoint PostgreSQL knows by name is present in `self.savepoints` under a string equal to what a later `ROLLBACK TO` / `RELEASE` produces.
Name strings come from `postgres_identifier` (`proxy/schema/middleware.rs:1037-1043`): ASCII lowercasing for unquoted identifiers, verbatim for quoted, no length truncation.
Whether PostgreSQL's own identifier normalisation (length limit, non-ASCII case folding) yields the same equivalence classes is not established in this repository.
When PostgreSQL confirms a `ROLLBACK TO` that L338 does not find, the settings are left as they were and `failed` stays as it was (I7).
Established by: nothing found.

A7. `SAVEPOINT` is confirmed only inside a transaction that will end through L328, L377 or L417, so `savepoints` is bounded by the transaction.
The vector has no cap (L330); its length within one transaction is the number of confirmed `SAVEPOINT` commands not yet truncated.
Established by: nothing found for a bound; clearing established at L412 and L417.

A8. All transaction ends are visible to `ConnectionSettings` as a `Commit`/`Rollback` command (L324), an Idle `ReadyForQuery` (L373), or `DISCARD ALL` (L356).
Commands classified `Other` (L183-L185) that end or alter a transaction have no effect here.
`Begin` (L364) does not re-baseline.
Established by: `ready_for_query` L372-L379 for the Idle case; nothing found for transaction boundaries that occur without an Idle `ReadyForQuery` and without a top-level `Commit`/`Rollback` statement.

A9. `failed` reflects PostgreSQL failure only.
Set by `execution_failed` (L368-L370), called from `restore_failed_preparation` (`mod.rs:264`) on `ErrorResponse` via `postgres_failed` (`lifecycle.rs:478`).
`local_execution_failed` (`lifecycle.rs:490-498`) does not set it.
After a Proxy-local failure, later `CommandComplete` messages on the discard path still reach this function (`backend.rs:219`, `lifecycle.rs:453-454`), and their `Change` commands are applied.

A10. `DISCARD ALL` is never confirmed while earlier unconfirmed-as-committed changes exist in the same implicit transaction.
`DiscardAll` drops the baseline and `failed` (I8), so changes applied before it in the same transaction cannot be restored by a later failure.
Established by: nothing found in this repository; it rests on PostgreSQL's handling of `DISCARD ALL` in multi-statement and pipelined contexts.

A11. Uncorrelated `CommandComplete` messages carry no settings effect.
`mod.rs:1573-1575` returns before calling when `operation` is `None`.
Established by pg-proto's operation correlation, whose source is not on disk.

A12. Projection and confirmation agree.
`may_change_effective` (L189-L209) and `project_settings_effects` (`settings_protocol.rs:47-61`) call this function with `success_tag()`: `"ROLLBACK"` for `Rollback`/`RollbackTo`, `""` otherwise (L211-L217), plus one extra `"ROLLBACK"` projection for `Commit` (L203-L207).
`may_change_effective` returns `false` before projecting for `Other | Begin | Savepoint | Release` (L190-L195).
The projected clone carries `savepoints`, `transaction_start` and `failed` from the confirmed state at clone time; commands still in flight at that time are not in it for the `execute` path, which projects against `connection` directly (`settings_protocol.rs:251`).

## 8. State read and written

Written: `keyset` (L340, L352, L358, L361, and via L326/L328/L356), `unsafe_disable_mapping` (L341, L354, L359, L362, and via L326/L328/L356), `transaction_start` (L318, and via L326/L328/L356), `failed` (L343, and via L328/L356), `savepoints` (L330, L342, L348, and via L328/L356).

Read: all of the above; `failed` is never read in this function.

Readers of the result elsewhere: `Context::keyset_identifier` (`mod.rs:1638-1644`) and `Context::unsafe_disable_mapping` (`mod.rs:1630-1636`) through `current()` (L280-L282), and `EffectiveSettings::new` (`settings_protocol.rs:27-32`).

Ordering relative to row decryption: on the normal path buffered encrypted rows are flushed (`backend.rs:230-252`) before `report_outcome` (`backend.rs:254`) reaches this function, so rows preceding the `CommandComplete` are decrypted under the pre-command settings (comment at `lifecycle.rs:444`).

## 9. Cross-function interaction with `ready_for_query` and `execution_failed`

- Error inside an explicit transaction: `failed = true` (L369); `ready_for_query(FailedTransaction)` does nothing (L373); the eventual `COMMIT`/`ROLLBACK` decides at L325.
- Error in an implicit transaction: `failed = true`; `ready_for_query(Idle)` restores the baseline (L374-L376) and ends (L377).
- Success in an implicit transaction: `ready_for_query(Idle)` ends without restore (L377), dropping `SET LOCAL` overrides.
- `RollbackTo` found clears `failed` (L343), so a later Idle `ReadyForQuery` does not restore.

## 10. Open questions

See the structured record; each is phrased as an item to inspect.
