# Backend::intercept_backend

File: `packages/cipherstash-proxy/src/postgresql/middleware/backend.rs`, L156-L377.
Helpers in the same file that it calls directly: `report_outcome` L89-L112, `decryption_failure` L122-L128, `error_response` L114-L116, `complete_describe` L118-L120, `flush_encrypted_rows` L473-L567, `check_column_config` L569-L600, `parameter_description_handler` L602-L653, `row_description_handler` L662-L697, `data_row_handler` L733-L752, `error_response_handler` L410-L425, `is_deallocation` L763-L766, `removes_all_statements` L769-L772, `execution_outcome` L78-L86.

Citation prefixes used below:

- `B` = `postgresql/middleware/backend.rs`
- `MM` = `postgresql/middleware/mod.rs`
- `L` = `postgresql/context/lifecycle.rs`
- `C` = `postgresql/context/mod.rs`
- `SP` = `postgresql/context/settings_protocol.rs`
- `S` = `postgresql/context/settings.rs`
- `P` = `postgresql/context/portal.rs`
- `SM` = `proxy/schema/middleware.rs`
- `DR` = `postgresql/rewrite/data_row.rs`
- `Z` = `proxy/zerokms/zerokms.rs`
- `EH` = `postgresql/error_handler.rs`

## 1. Purpose

Single dispatcher for every backend (PostgreSQL to client) message on one connection.
It receives `(Option<OperationId>, BackendMessage)` and returns one `BackendMiddlewareOutput`: `Forward(msg)`, `Suppress(msg)`, or `Expand(Vec<msg>)` (B215, B227, B290, B295, B372, B375, B127).
It is the only place where PostgreSQL's confirmation of an operation is turned into committed Proxy state: connection settings, the settings journal, the per-operation table, the schema execution queue, and the decrypted-row stream.

## 2. Entry and callers

- `Backend::intercept` (B148-B154) is a one-line wrapper and the only non-test caller.
- `CipherStashMiddleware::backend_operation` (MM65-MM76) calls `intercept` with `operation.map(Into::into)`; the operation id is whatever pg-proto supplies.
- `Frontend` and `Backend` live in the same `CipherStashMiddleware` value and both entry methods take `&mut self` (MM15-MM18, MM54-MM76), so the two adapters never run concurrently on one connection.
  They do interleave message by message.
- `Backend` holds its own `Context` clone (B68, MM39-MM40); `protocol_state` and every `SchemaMiddleware` field are `Arc`-shared between the two clones (C67, SM313-SM330).

## 3. Inputs and their origin

| Input | Origin | Checked here |
|---|---|---|
| `operation: Option<OperationId>` | pg-proto correlation (MM70-MM74) | Never validated here; each callee looks it up in `operations` and several return `UnknownOperation` when absent. |
| `protocol_message` variant | Upstream PostgreSQL via pg-proto framing | Pattern match only. |
| `CommandComplete(tag)` bytes | PostgreSQL | Compared byte-for-byte to three literals after stripping one trailing NUL (B763-B772); passed as lossy UTF-8 to the settings layer (B98). |
| `DataRow.columns` | Stored table content | Byte lengths summed (B280-B285); content parsed only inside `flush_encrypted_rows`. |
| `ReadyForQuery(status)` | PostgreSQL | Passed through to `became_ready` (B200, B222, B359). |
| `ErrorResponse` fields | PostgreSQL | Logged at debug (B410-B425); never inspected for SQLSTATE or severity. |

## 4. Block-by-block

### 4.1 Clone (B161)

`outbound_message` starts as a clone of the incoming message.
Every later arm either leaves it or replaces it.

### 4.2 SQL deallocation acknowledgement (B163-B170)

Runs when the message is `CommandComplete(tag)` and `operation` is `Some`.
`is_deallocation(tag)` is true for exactly `DEALLOCATE`, `DEALLOCATE ALL`, `DISCARD ALL` (B763-B766).
`complete_deallocation(operation, all)` (C861-C905):

- Returns `Ok(())` without change when `operations` has no entry for the operation (C867-C873) or its `deallocations` queue is empty (C874-C876).
  This callee does not fail closed on an unknown operation, unlike the others in this function.
- Pops the front queued `Deallocation`; when its kind disagrees with `all`, clears the queue, logs a warning, removes nothing (C877-C887).
- Otherwise removes the named statements from `prepared_statements` and from `settings_protocol.prepared` when the stored Parse operation is older than this operation (C891-C898, SP485-SP510).

This block runs before the passthrough check (B187) and the discard check (B218), so it applies in all three modes.
The tag text alone selects the block; the queued entry was written by the Frontend (`set_deallocations` C829-C846, `set_execute_for_portal` C698-C704).

### 4.3 Parse/Bind acknowledgement (B172-B179)

For `ParseComplete` or `BindComplete` with `Some(operation)`, `preparation_acknowledged` (L408-L427):

- Removes the operation's journal entry (`prepared_or_bound`, SP326-SP330) and releases metrics scopes that entry alone referenced (L414-L416).
- Removes `operations[operation]` and returns `UnknownOperation` when absent (L417-L420).
- The journal removal at L414 happens before the existence check at L417-L420, so the `Err` path leaves the journal entry already removed.

With `operation == None` nothing is acknowledged and the message proceeds.

### 4.4 Close acknowledgement (B181-B185)

For `CloseComplete` with `Some(operation)`, `close_acknowledged` (L429-L442) removes the journal entry and releases unreferenced metrics.
It does not touch `operations` and has no `UnknownOperation` path.

### 4.5 Passthrough mode (B187-B216)

`is_passthrough()` is `encrypt_config().is_empty() || config.mapping_disabled()` (C1753-C1755).
It is evaluated once per message, against the connection's current `SchemaMiddleware.encrypt_config` (SM387-SM389).
It does not read the per-connection `unsafe_disable_mapping` setting.

When true:

1. `report_outcome(operation, &msg, false)` (B188); see 4.9.
2. On `ReadyForQuery`, `became_ready` (B199-B201).
3. A `Some` replacement overwrites `outbound_message` with an `ErrorResponse` (B203-B205), with no check on the message variant.
4. `RowDescription` or `NoData` with `Some(operation)` calls `complete_describe` (B206-B213).
5. Returns `Forward(outbound_message)` (B215).

Not executed in this mode: the `discard_execution` test (B218), the flush of `encrypted_rows` (B230-B252), `data_row_handler`, and both description rewrites.
A `DataRow` is therefore forwarded byte-identical regardless of what portal its operation holds.
If `encrypted_rows` is non-empty when `is_passthrough()` becomes true, those rows stay buffered until a later non-passthrough, non-`DataRow` message arrives.
If `discard_execution` is true when `is_passthrough()` becomes true, it stays set and is not consulted.

Writers of `encrypt_config` that can change the result between two calls:

- `adopt_latest` (SM392-SM402), reached from the Backend at Idle readiness (SM456-SM458, C1719-C1721, SM427-SM430) and from the Frontend's `before_statement` (SM438-SM446) and `prepare_schema_for_statement` (C1421-C1433).
- `apply_ddl` on a confirmed DDL (SM928-SM936), reached from `report_outcome` in this same function through `execution_succeeded` (SM788-SM790).
- Full rollback and rollback-to-savepoint restores (SM753, SM771-SM772).

The Frontend's `before_statement` adoption is gated on `transaction_active == false`, no local changes, no pending publication and `in_flight_ddl == 0` (SM439-SM443).
`transaction_active` is set true by the Frontend at each Execute and simple Query (SM514, SM527) and overwritten at each readiness by the Backend (SM451-SM452).

### 4.6 Discard mode (B218-B228)

Entered when `discard_execution` is true and the connection is not in passthrough.

1. `report_outcome(operation, &msg, true)?` (B219); the returned replacement is dropped.
2. On `ReadyForQuery(status)`: clear the flag (B221), `became_ready` (B222), return `Forward(ReadyForQuery(status))` (B223-B225).
   The flag is cleared before `became_ready`, so an `Err` from `became_ready` leaves it cleared.
3. Every other message returns `Suppress(msg)` (B227).

Consequences that follow from the lines above:

- The flag is set only by Proxy-local failures (B126, B237); PostgreSQL has not failed and keeps answering the rest of the batch.
- Outcomes of those later messages are still committed with `discarded = true`: settings changes (L453-L455), schema execution results (L456), SQL deallocations (B163-B170), Parse/Bind/Close acknowledgements (B172-B185).
- `ErrorResponse`, `NoticeResponse`, `ParameterStatus`, `NotificationResponse`, `RowDescription`, `ParameterDescription`, `NoData` and all `DataRow`s are suppressed until the first `ReadyForQuery`.
- `complete_describe` is not called for suppressed `RowDescription`/`NoData`; the describe context stays in `operations` until `ready_for_query` removes ids `<= boundary` (C984-C1002), which requires a `Some` boundary (C945-C947).
- The forwarded `ReadyForQuery` carries PostgreSQL's real transaction status.

### 4.7 Flush prefix (B230-B252)

Condition: the message is not a `DataRow` and `encrypted_rows` is non-empty (B230-B231).
Any non-row message triggers it, not only the three terminal messages.

- Success: `prefix` holds the decrypted rows (B235) and is emitted ahead of the current message at B374-B375.
- Failure (B236-B248), in this order:
  1. `discard_execution = true` (B237).
  2. `local_execution_failed(encrypted_rows_operation)?` (B238-B239), which is `finish_execution(op, Failed)` (L490-L498).
     Its `Ok(Some(replacement))` is dropped by the `?;`.
     Its `Err` (for example `UnknownOperation`, C342-C344) propagates with `discard_execution` already true.
  3. Only when `operation.is_some()`: `report_outcome(operation, &msg, true)?` (B244-B246).
  4. `decryption_failure(err)` clears the buffer fields, sets the flag again, returns `Expand([ErrorResponse])` (B122-B128).

On this path the triggering message is not forwarded.
If the triggering message is `ReadyForQuery`, `became_ready` is not called and `discard_execution` stays true until the next `ReadyForQuery`.
`report_outcome` returns `Ok(None)` for `ReadyForQuery` (B110), so step 3 does nothing for it.

`flush_encrypted_rows` takes ownership of the buffer before any fallible step (B474-B476), so every `Err` from it drops the held rows.

### 4.8 Ordering established by 4.7 and 4.9

On the normal path the flush (B234) precedes `report_outcome` (B254).
A `CommandComplete` that commits a settings change (L453-L455) is applied after the rows that preceded it were decrypted.
The keyset used for decryption is read at flush time from confirmed `ConnectionSettings` (C1681, C1638-C1644), not captured at Bind or Execute.

### 4.9 report_outcome (B89-B112) and what it commits

Called once per message on the passthrough (B188), discard (B219) and normal (B254) paths, and zero or one time on the flush-failure path (B244-B246).

`CommandComplete(tag)` -> `execution_completed(op, Command(lossy tag), discarded)` (B96-B100, L447-L471):

1. `settings_execution_succeeded(op, tag)` (C1568-C1606): returns immediately for `op == None` (C1573-C1575); otherwise pops the next pending `SettingsCommand` for that operation (SP287-SP293) and applies it to `ConnectionSettings` (C1582, S317-S366).
   A popped `DiscardAll` also prunes `prepared_statements`, `portals`, `portal_operations` to what `settings_protocol` still holds (C1583-C1599, SP294-SP323).
   The command applied is positional within the operation's queue; the tag text is used only to tell `ROLLBACK` from `COMMIT` (S324-S329).
2. `report_schema_execution_succeeded()` (L456, C1394-C1396, SM704-SM815): pops the front of the connection-wide FIFO `executions` if it is an `Execute` entry (SM950-SM960) and applies its intent.
   This call takes no operation id and runs for `op == None` as well.
3. `op == Some`, `discarded == true`: `discard_operation` (C1026-C1043) removes `operations[op]` unconditionally, no error when absent.
4. `op == Some`, `discarded == false`: `finish_execution(op, Completed)` (C709-C735, C337-C395).

`EmptyQueryResponse` -> same without step 1 (B101-B104, L453).
`PortalSuspended` -> same without step 1, outcome `Suspended` (B105-B108, L463-L465).
Step 2 runs for `PortalSuspended`, so a suspended portal pops and applies one schema execution entry (SM704-SM705; test B1227-B1244).

`ErrorResponse` -> `postgres_failed(op, discarded)` (B109, L473-L488):

1. `settings_execution_failed(op)` (C1608-C1619) -> `restore_failed_preparation(op)` (C216-C266).
   `settings_protocol.failed(op)` returns empty for `None` (SP337-SP339).
   `self.settings.execution_failed()` runs unconditionally (C264), setting `failed = true` (S368-S370) for every `ErrorResponse`, correlated or not.
   At the next Idle readiness `failed` restores the transaction-start snapshot (S372-S378, S381-S386).
2. `report_schema_execution_failed()` (L479, SM939-SM947): sets `temporary.failed` and pops one front `Execute` entry.
3. `discard_operation` or `finish_execution(op, Failed)` as above.

All other variants -> `Ok(None)` (B110).

`finish_execution` paths (C337-C395):

- Unknown operation -> `Err(UnknownOperation)` (C342-C344).
- `Completed` on an execute with `completes_at_readiness` (simple Query, C670) -> returns without removing anything (C345-C358).
- No execute and outcome is not `Failed` -> `Err(OperationWithoutExecute)` (C359-C362).
- `replacement_error` is taken only for `Failed` (C363-C367).
- The operation is removed when `Failed` or when no describe is pending (C368-C372).
- `Suspended` stores `(operation, execute)` in `suspended_executions` (C377-C381); the `unwrap` at C378 is guarded by C360-C362.
- Outside the lock, a finished non-suspended execute on the unnamed portal closes that portal if it is still the same `Arc` (C724-C732, C1173-C1195).

Steps 1 and 2 of both `execution_completed` and `postgres_failed` run before the fallible step 3/4.
When step 4 returns `Err`, the settings and schema mutations of steps 1 and 2 have already landed (confirmed by the test at B1022-B1041, which asserts the schema queue moved although the call returned `Err`).

### 4.10 Debug keyset read (B256-B259)

Only when DEBUG is enabled for target `CONTEXT`.
`keyset_identifier()?` fails only on a poisoned lock (C1638-C1644).

### 4.11 Per-message arms (B261-B369)

`DataRow(row)` (B262-B302):

- `data_row_handler(operation)` (B733-B752): for `None` returns `Ok(false)`; for `Some` calls `get_portal_from_execute`, which errors on unknown operation or missing execute (C1290-C1307); returns `Ok(true)` only for `Portal::Encrypted`.
- `Ok(false)`: falls out of the match and is forwarded unchanged (B371-B372).
  `prefix` is always empty here because the flush block skips `DataRow` (B230).
  A non-encrypted row is therefore forwarded ahead of any rows still held in `encrypted_rows`.
- `Ok(true)` with `operation == None` (B266-B270): `decryption_failure(HeldDataRowMissingOperation)`.
  `data_row_handler` cannot return `true` for `None` (B735-B745), so this branch has no reachable input with the current callee.
- Operation continuity (B271-B279): `encrypted_rows_operation.replace(operation)` and, when the previous value differs, `decryption_failure(HeldDataRowOperationMismatch)`.
  `local_execution_failed` is not called on this path, so neither operation's execute context is finished here.
- Accounting (B280-B286): adds the sum of non-NULL column byte lengths, pushes a clone of the row.
- Under both limits (`len < 4096` and `bytes < 64 MiB`, B75-B76, B287-B288): `Suppress(DataRow(row))` (B290-B292).
- Otherwise flush (B294-B300): `Expand(rows)` on success; on failure `local_execution_failed(Some(operation))?` then `decryption_failure`.
- The limits are tested after the push, so one row of any size is accepted before the flush.

Terminal messages (B306-B314): logs the outcome; substitutes the replacement when `Some`.
`finish_execution` returns a replacement only for `Failed` (C363-C367), so with the current callee this substitution never fires for these three variants (test B1103-B1125).

`ErrorResponse` (B315-B322): debug log; substitutes the stored Proxy diagnostic when `operation.is_some()` and a replacement was returned.
A replacement exists only when the Frontend stored one on that same operation (C1045-C1058, L208-L229, L283-L304, L384-L406).

`ParameterDescription(types)` (B326-B330): `parameter_description_handler` (B602-B653).
`get_statement_from_describe` errors on an unknown operation (C1272-C1288).
When a statement is present, the outgoing OID list is rebuilt with one entry per client-side param (`statement.param_columns`), taking the server OID for native params by `output_params` position and `UNSPECIFIED_TYPE_OID` when no position matches (B618-B638).
No `complete_describe` in this arm.

`RowDescription(description)` (B336-B343): `row_description_handler` (B662-B697) then `complete_describe` for `Some(operation)`.
`get_statement_for_operation` errors on an unknown operation and otherwise prefers the describe's statement, then the execute's encrypted portal statement (C1242-C1270).
Type OIDs are overwritten positionally by `zip` (B683-B689); the field count and projection count are not compared.
`complete_describe` (C528-C548): unknown operation -> `Err`; no describe but an execute present -> `Ok`; neither -> removes the operation and returns `Err(UnknownDescribe)`.

`NoData` (B346-B350): `complete_describe` for `Some(operation)`.

`ReadyForQuery(status)` (B354-B360): `became_ready(operation, status)` (L500-L519):

1. `ready_for_query(status, operation)` (C931-C1015): `settings.ready_for_query` (S372-S379); `settings_protocol.ready` (SP453-SP482); with `operation == None` it returns here (C945-C947) without pruning `operations`, portals or suspended executions.
   With `Some(boundary)` it removes every operation with id `<= boundary` (C984-C1002) and, when Idle, every portal and suspended execution owned by an operation `<= boundary` (C949-C982).
2. When Idle: `publish_schema_if_changed().await?` (C1718-C1734).
   When publication is needed it sends a `ReloadCommand` and awaits the oneshot reply (C1695-C1715); a false or dropped reply returns `Err(SchemaCouldNotBeLoaded)` (C1728-C1730).
   On that `Err` step 3 is skipped and the `ReadyForQuery` is not forwarded.
3. `schema_ready_for_query` (SM449-SM459): drops queued executions up to the next `ReadyBoundary` (SM963-SM982), sets `transaction_active`, adopts the latest snapshot when Idle and nothing needs publication.

Default arm (B362-B368): debug log, forwarded unchanged.
This covers every other backend message, including the COPY family, `NoticeResponse`, `ParameterStatus`, `NotificationResponse`, `ParseComplete`, `BindComplete`, `CloseComplete`.

### 4.12 Emit (B371-B376)

Empty prefix -> `Forward(outbound_message)`; otherwise `Expand(prefix + [outbound_message])`.

## 5. flush_encrypted_rows as a callee (B473-B567)

Facts the dispatcher depends on:

- Resolves the portal from `encrypted_rows_operation` through `get_portal_from_execute` (B478-B481); a non-`Encrypted` portal, or no operation, returns `HeldDataRowsNotEncrypted` (B482-B488).
  The lookup happens at flush time, so the operation's execute context must still exist then.
- `result_column_count` is the first row's column count (B491-B494).
- `as_ciphertext` zips each row's columns with `projection_columns` and yields `min(row columns, projection columns)` entries per row (DR21-DR44).
- A configured, non-NULL column whose bytes fail to parse yields `None` after an error log (DR26-DR41); `check_column_config` accepts `(Some(col), None)` (B585-B586); `rewrite` leaves a column untouched when its plaintext slot is `None` (DR57-DR62).
  That column's stored bytes go to the client unchanged.
- `check_column_config` zips `projection_columns` with the flat ciphertext list of all rows (B574), so it examines at most `projection_columns.len()` entries, which is the first row of the batch.
- `Context::decrypt` forwards to the service without comparing output length to input length (C1677-C1683); `Context::encrypt` does compare (C1666-C1673).
  `ZeroKms::decrypt_eql` returns `ciphertexts.len()` entries on both `Ok` paths (Z444-Z446, Z461-Z468).
- `plaintexts.chunks(result_column_count).zip(rows)` (B547) pairs chunks with rows; `zip` ends at the shorter side.
  `chunks(0)` panics per the standard library contract; nothing between B491 and B547 rejects a first row with zero columns.
- Format codes come from the portal; with more than one code the stored list is used as is (P56-P68) and zipped with each chunk (B553-B560).

## 6. State touched

Backend-local:

- `encrypted_rows`, `encrypted_rows_operation`, `encrypted_rows_bytes`: written B271-B286, B474-B476, B123-B125.
- `discard_execution`: set B126, B237; cleared B221.

Shared with the Frontend through `Context`:

- `protocol_state.settings`: C1582, C264, C941.
- `protocol_state.settings_protocol`: SP287-SP325, SP326-SP330, SP333-SP432, SP453-SP482, SP485-SP510.
- `protocol_state.operations`: L417-L420, C371, C541-C546, C993, C1034.
- `prepared_statements`, `portals`, `portal_operations`, `suspended_executions`, `statement_metrics`: C891-C898, C1594-C1598, C216-C266, C379-C387, C949-C982, C1191-C1195.
- `SchemaMiddleware`: `executions`, `in_flight_ddl`, `dirty`, `unmodelled`, `savepoints`, `temporary`, `encrypt_config`, `resolver`, `transaction_active` (SM704-SM815, SM939-SM947, SM449-SM459, SM392-SM402).

Process-wide:

- `CommittedSchemaStore` publication flag via `mark_publication_pending` (C1724-C1726, SM415-SM417).
- `ReloadCommand` channel (C1697-C1699).
- `ZeroKms.cipher_cache` via `decrypt` (Z400).
- Prometheus counters and histograms (B734-B748, B523, B541-B543).

## 7. Invariants

1. Deallocation, Parse/Bind and Close acknowledgements run before any mode branch (B163-B185 precede B187 and B218).
2. `report_outcome` is called at most once per message; exactly once on passthrough, discard and normal paths (B188, B219, B254); zero or one time on the flush-failure path (B244-B246).
3. On the normal path buffered rows are flushed before the triggering message's outcome is committed (B234 before B254).
4. A row enters `encrypted_rows` only with `Some(operation)` whose execute holds `Portal::Encrypted` (B265-B270, B735-B745).
5. All rows in `encrypted_rows` share one operation id (B271-B279).
6. After each push either the function returns with `len < 4096` and `bytes < 64 MiB`, or it flushes (B287-B300).
7. `discard_execution == true` implies `encrypted_rows` is empty at function exit: every site that sets it has cleared or taken the buffer (B123-B126; B237 follows the take at B474-B476), and the discard branch returns before the push (B218-B228).
8. In discard mode only `ReadyForQuery` reaches the client (B220-B227).
9. `discard_execution` is cleared only at B221.
10. A stored Proxy diagnostic replaces a message only when `finish_execution` ran with `Failed` (C363-C367), which `report_outcome` does only for `ErrorResponse` (B109, L484).

## 8. Assumptions and where each is established

1. pg-proto gives every `DataRow` of an encrypted execution the `Some(operation)` of its Execute or Query.
   Established by: nothing found in this repository.
   With `None`, the row takes the `Ok(false)` branch and is forwarded unchanged (B735-B750, B371-B372).
2. The operation's execute context still exists when the first row arrives and when the flush runs.
   Established by: Frontend registration (C653, C671, C703); removed by `finish_execution` (C371), `discard_operation` (C1034), `ready_for_query` (C993), `preparation_acknowledged` (L417-L420).
3. `is_passthrough()` at Backend time agrees with the classification the Frontend used when it built the portal.
   Established by: nothing found in this function; partly by the `transaction_active` gate in `before_statement` (SM439-SM443).
   The Backend's own Idle adoption (SM456-SM458) and confirmed-DDL overlay (SM933-SM935) change `encrypt_config` while later pipelined operations already hold portals.
4. A row sequence is always ended by `CommandComplete`, `PortalSuspended` or `ErrorResponse` before any `ReadyForQuery` or any row of another operation.
   Established by: the PostgreSQL protocol and pg-proto ordering; nothing found in this function.
   Line that would enforce it for unbuffered rows: B301 (a non-encrypted row is forwarded while rows are held).
   Line that would enforce it for readiness: B236-B247 (a failed flush triggered by `ReadyForQuery` skips `became_ready`).
5. Uncorrelated terminal and error messages belong to the front of the schema execution queue.
   Established by: nothing found; `execution_succeeded`/`execution_failed` take no operation id (L456, L479, SM704, SM939).
   The flush-failure path is the one place where an uncorrelated message does not advance the queue (B244).
6. The Nth `CommandComplete` of an operation corresponds to the Nth queued settings command and the Nth queued deallocation.
   Established by: Frontend queue order (SP262-SP286, C829-C846) plus PostgreSQL emitting one tag per statement; the deallocation path cross-checks tag kind (C877-C887), the settings path does not (SP287-SP293).
7. `OperationId` ordering matches send order.
   Established by: derive `Ord` over `pg_proto::OperationId` (`postgresql/mod.rs` L20-L25); pg-proto's ordering is not visible.
   Relied on at C954, C971, C987, SP459-SP470, SP300-SP302, SP489.
8. `ReadyForQuery` carries the `Some(operation)` of the Sync or Query that it answers.
   Established by: nothing found; with `None`, `ready_for_query` skips all pruning (C945-C947).
9. An `ErrorResponse` correlated to `Some(operation)` targets an operation that has an `operations` entry.
   Established by: Frontend registrations (L86, L113, L250, C509, C518-C520, C653, C671, C1053-C1056).
   `sync` creates no entry (L365-L368, C1559-C1566).
   Without an entry `finish_execution` returns `UnknownOperation` (C342-C344) after settings and schema state were already changed (L478-L479).
10. Every `ErrorResponse` means the current transaction failed.
    Established by: nothing found; `settings.execution_failed()` is unconditional (C264).
11. A middleware `Err` terminates the connection.
    Established by: nothing found in this repository; pg-proto behaviour.
    Every `?` in this function relies on it, because state mutated before the `Err` is not rolled back.
12. `projection_columns.len()` equals each held row's column count, and that count is non-zero.
    Established by: nothing found in `flush_encrypted_rows` (B491-B508, B547); depends on how the Frontend builds `Statement.projection_columns`.
13. Every row in a flushed batch carries ciphertexts for the same columns as the first row.
    Established by: nothing found; `check_column_config` covers the first row only (B574).
14. The `EncryptionService` returns one plaintext slot per ciphertext slot.
    Established by: `ZeroKms::decrypt_eql` (Z444-Z446, Z461); not checked in `Context::decrypt` (C1677-C1683) or at B547.
15. The keyset confirmed at flush time is the keyset the rows were written under or the one the client is entitled to.
    Established by: `ZeroKms::decrypt_eql` cipher scoping (Z396-Z400); the read path applies no ciphertext-keyset comparison (Z401-Z408 applies only to `AllEntries`).
16. `Suppress(msg)` means the message is not delivered to the client, and `Expand` delivers in vector order.
    Established by: nothing found; pg-proto.
17. pg-proto bounds the size of one `DataRow`.
    Established by: nothing found; the 64 MiB test runs after the push (B286-B288).

## 9. External and black-box calls

- pg-proto: supplies `operation`, consumes the returned output, decides what an `Err` does.
  Source is not on disk.
- `Context::decrypt` -> `EncryptionService::decrypt` -> ZeroKMS: awaited inside `flush_encrypted_rows` (B522).
  The whole middleware is borrowed mutably during the await (MM65-MM76), so no Frontend message for this connection is processed until it returns.
- `Context::reload_schema`: sends on the unbounded channel and awaits a oneshot (C1695-C1715) inside `became_ready`.
  The same borrow holds during this await.
  Other connections and the reloader tasks run during it and can change `CommittedSchemaStore`.

## 10. Open questions

1. Unclear what pg-proto does with a middleware `Err`; need to inspect pg-proto 0.12.0 `IntermediaryMiddleware` driver loop.
2. Unclear which backend messages pg-proto correlates and to which frontend operation (`DataRow`, `ReadyForQuery`, `ErrorResponse` during Sync, asynchronous `NoticeResponse`/`ParameterStatus`/`NotificationResponse`); need to inspect pg-proto operation tracking.
3. Unclear whether pg-proto can call `frontend_operation` between two backend messages of one execution; need to inspect pg-proto scheduling, since that decides whether `before_statement` (SM438-SM446) can run between a held row and its flush.
4. Unclear whether `Context::close` creates an `operations` entry; need to inspect `lifecycle.rs` L345-L363.
5. Unclear which statement's projection the single unnamed portal holds for a multi-statement simple Query; need to inspect `frontend.rs` L476-L531, since `data_row_handler` and `row_description_handler` consult that one portal for every statement's rows.
6. Unclear whether a portal can be `Encrypted` while PostgreSQL returns zero-column rows or more columns than `projection_columns`; need to inspect `Statement::has_projection` and the Bind path at `frontend.rs` L1049-L1050.
7. Unclear what the Frontend sends upstream for a refused Execute or Query (L384-L406) and whether PostgreSQL always answers it with an `ErrorResponse` on the same operation; a stored diagnostic reaches the client only then (C363-C367).
8. Unclear how `CopyOutResponse`/`CopyData` for a table with encrypted columns are treated upstream of the default arm (B362-B368); need to inspect the Frontend COPY checks.
9. `PreparedStatements::deallocation_names` and `deallocate` (C891-C897) were not read; need to inspect them for which names an `AllStatements` deallocation removes relative to the operation id.
10. `close_portal_if_current` was read only to C1195; need to inspect the remainder for the metrics release path.
