# Frontend::parse_handler

File: `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`, L804-L902.

The analysis format and worked-example resources named in the task (`ANALYSIS_FORMAT.md`, `FUNCTION_MICRO_ANALYSIS_EXAMPLE.md`, `DOMAIN_NOTES.md`) were not present in the repository, so this document uses its own section layout.

Citation prefixes used below:

- `F:` `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`
- `LC:` `packages/cipherstash-proxy/src/postgresql/context/lifecycle.rs`
- `CM:` `packages/cipherstash-proxy/src/postgresql/context/mod.rs`
- `SP:` `packages/cipherstash-proxy/src/postgresql/context/settings_protocol.rs`
- `PS:` `packages/cipherstash-proxy/src/postgresql/context/prepared_statements.rs`
- `ST:` `packages/cipherstash-proxy/src/postgresql/context/statement.rs`
- `COL:` `packages/cipherstash-proxy/src/postgresql/column_mapper.rs`
- `PA:` `packages/cipherstash-proxy/src/postgresql/parser.rs`
- `SM:` `packages/cipherstash-proxy/src/proxy/schema/middleware.rs`
- `MW:` `packages/cipherstash-proxy/src/postgresql/middleware/mod.rs`
- `EM:` `packages/eql-mapper/src/eql_mapper.rs`
- `TCS:` `packages/eql-mapper/src/type_checked_statement.rs`
- `PP:` `packages/eql-mapper/src/param_plan.rs`
- `BR:` `packages/cipherstash-proxy/src/postgresql/rewrite/bind.rs`

## Purpose

Handles one extended-protocol `Parse` message from an authenticated client.
It registers the statement name speculatively, parses the SQL text, decides a route (map, forward, or refuse), encrypts literals and rewrites the SQL when the route is `Map`, rewrites the declared parameter type OIDs, and installs a `Statement` (or nothing) under the statement name.
The return value tells the caller whether to forward the original bytes (`Ok(None)`, F:900), a rewritten `Parse` (`Ok(Some)`, F:898), or to refuse (`Err`).

## Inputs and who controls them

- `operation: OperationId` (F:806). Supplied by `pg-proto` through `MW:59-62`. Not client-controlled as a value; the crate that allocates it is not in this repository.
- `message: Parse` (F:807). Three client-controlled fields are read:
  - `message.statement` (statement name, bytes) at F:812 and F:820.
  - `message.query` (SQL bytes) at F:809, F:815.
  - `message.parameter_types` (`u32` OIDs) at F:829.
- Implicit inputs read through `self.context`:
  - Per-connection `ConnectionProtocolState` behind `Arc<RwLock>` (CM:154-163).
  - Effective connection settings (`unsafe_disable_mapping` at CM:1630-1636, keyset at CM:1638-1644).
  - Schema middleware state: resolver (SM:373-378), encrypt config (SM:387-389), unmodelled flag (SM:433-435), in-flight DDL, publication flags (SM:420-424).
  - Operator config: `mapping_errors_enabled` (CM:1798-1800), `default_keyset_id` (CM:1810-1815).

## Caller and preconditions established by it

Single production caller: `Frontend::intercept` at F:256-275.

Before reaching F:258 the caller has established:

- The message is not `Terminate` (F:176-178).
- `self.failed_extended_batch` is false and `context.discarding_extended_messages()` is false (F:179-187). When either is true, a `Parse` is suppressed and `discard_operation` is called instead (F:185-186).
- Operator-level `mapping_disabled()` is false (F:189). When it is true, `Parse` goes to `context.unmapped_parse` (F:203-205) and the function returns at F:210 without calling `parse_handler`.

The caller clones the whole `Parse` before the call (F:257) for use on the refusal path.

What the caller does with each outcome:

- `Ok(Some(mapped))`: forwards `mapped` (F:259-261).
- `Ok(None)`: forwards the original message (`outbound_message` was initialised to a clone at F:213; F:262).
- `Err(err)`: converts to a diagnostic (F:269), calls `context.refuse_parse(operation, response)?` (F:270), sets `failed_extended_batch = true` (F:271), and replaces the outbound message with a substitute `Parse` (F:272).

## Block-by-block walk

### 1. Capture original bytes and begin the speculative preparation (F:809-813)

`original_query` is a clone of the client bytes, used only for the change comparison at F:890.

`context.begin_parse(operation, name)` (LC:91-122) does the following, in order:

1. Allocates a new metrics scope id from an atomic counter and inserts it into `statement_metrics` (CM:550-559).
2. Marks the scope `protocol = Extended` (LC:93).
3. Under one write lock (LC:95-98):
   - Takes a snapshot of everything currently stored under `name` (CM:271-279: statement, metrics scope, parse operation, deallocation).
   - Records that snapshot in the settings-protocol journal keyed by `operation`, together with the current `SettingsProtocol.prepared` entry for the name (SP:139-154). `journal.insert` replaces any existing journal entry for the same operation id (SP:145).
   - Removes the whole existing `PreparedStatements` entry for `name` (LC:103, CM:296-303, PS:86-88). The previous `Statement`, deallocation, and parse operation are gone from live state at this point, before PostgreSQL has seen the new Parse.
   - Sets `records_statement_metrics = false` on the new scope (LC:104-106).
   - Creates a fresh entry for `name` with the new metrics scope and `parse = operation` (LC:107-112, PS:65-75). The entry's `statement` field is `None` (PS:17-25).
   - Sets `operations[operation].preparation = Parse { name, metrics, intent: false }` (LC:113-117).
4. Records metrics for the removed scope when nothing else references it (LC:120, CM:298-302).

Consequence relied on later: from F:812 onward `prepared_statements.contains(name)` is true (PS:49-51), with `statement == None` until `finish_parse` runs.
`Context::is_prepared` reads exactly that predicate (CM:1076-1082), and `bind_handler` uses it as its unknown-statement gate (F:1023).

Failure paths of `begin_parse`: only a poisoned lock (`ProtocolStateUnavailable`) at CM:553-555, LC:93, LC:95-98.
If it fails before LC:113, the caller's `refuse_parse` finds no `Preparation::Parse` and returns `UnknownOperation` (LC:218-225), which propagates out of `intercept` through the `?` at F:270.

### 2. Decode and parse the SQL (F:815-817)

`String::from_utf8_lossy(&message.query).into_owned()` (F:815) produces the text that every later analysis step uses.
Invalid UTF-8 sequences are replaced, so `statement_text` differs from `message.query` whenever the client bytes are not valid UTF-8.
The bytes forwarded on the no-rewrite path are the original bytes, not `statement_text` (F:890-900).

`SqlParser::parse_statement(&statement_text)?` (F:817, PA:20-36):

- Tokenises with `PostgreSqlDialect` and normalises `RESET` forms into `SET` tokens (PA:93-177). `SET [SESSION|LOCAL] ALL` is rejected at PA:108-113.
- If any `;`-separated segment starts with `COPY` and has `FROM` directly followed by `STDIN` (PA:184-213), each segment is parsed separately and **only the first resulting statement is returned**; the rest are discarded (PA:22-26).
- Otherwise calls sqltk `Parser::parse_statement()` (PA:28-30), which is an external crate (`sqltk = "0.11.0"`, root `Cargo.toml:49`) whose source is not in this repository.
- Increments `STATEMENTS_TOTAL` (PA:33).

The returned AST describes one statement.
Nothing in `parse_handler` compares the AST's extent with the full text.

On error, the function returns at F:817 with `begin_parse` state in place and `intent: false`.

### 3. Record SQL-level deallocation (F:818-821)

`to_deallocation(&statement)` (F:1458-1478) returns:

- `AllStatements` for unquoted `DEALLOCATE ALL` (F:1461-1462) and for `DISCARD ALL` (F:1473-1475).
- `Statement(name.to_ascii_lowercase())` for an unquoted name (F:1463-1468).
- `Statement(name)` for a quoted name (F:1469-1471).
- `None` otherwise (F:1476).

When `Some`, `set_statement_deallocation` writes it onto the entry for the *Parse's own* statement name (CM:800-811, PS:81-83).
This write happens before `prepare_parse`, so it is present on the entry even when a later step fails; the caller's `refuse_parse` then removes the whole entry (LC:227, CM:784-796).

### 4. prepare_parse (F:823-825, LC:124-173)

Steps, in order:

1. Reads `operations[operation].preparation` and requires `Parse { intent: false }`; anything else is `UnknownOperation` (LC:130-147).
2. `prepare_schema_for_statement().await?` (LC:148, CM:1421-1433):
   - When the connection is idle, has no local changes, and the shared store has publication pending (SM:420-424), awaits `reload_schema()`; a false result is `SchemaCouldNotBeLoaded` (CM:1426-1428); success calls `publication_succeeded` which adopts the latest snapshot (SM:427-430).
   - Then `before_statement()` adopts the latest committed snapshot only when not in a transaction, no local changes, no publication pending, and no in-flight DDL (SM:438-446). On every other combination the connection keeps its current resolver and encrypt config.
3. Only when `eql_mapper::requires_schema(statement)` is true (LC:149; EM:120-122: type-checked kinds or `COPY ... FROM <table>`): awaits `wait_for_schema_execution()` (CM:1390-1392) and then fails with `UnmodelledDdl` when `has_unmodelled_ddl()` (LC:151, CM:1408-1413).
4. `prepare_schema_statement(name, statement.clone())` (LC:153) inserts a schema intent for the name into the schema middleware's `prepared` map unconditionally (SM:485-488).
5. Writes `statement_type` and query fingerprint (computed from `statement_text`, the lossy-decoded text) to the metrics scope (LC:154-157).
6. `validate_setting(statement)?` (LC:158, LC:521-530): returns `UnexpectedSetKeyset` when the statement is a keyset setting change and the operator configured a default keyset. `SettingChange::parse` can also return an error (LC:523).
7. `prepare_settings_statement(operation, name, statement)?` (LC:159, CM:1440-1454): under a write lock, captures `EffectiveSettings` (keyset and mapping-disabled flag, SP:27-32) from the confirmed connection settings and stores `Prepared { operation, command, needs_mapping: requires_schema(statement), settings, literal_sensitive: false }` under the name (SP:160-185). `SettingsCommand::from_statement` can fail (SP:167). The journal entry inserted by `begin_parse` is kept (`entry().or_insert`, SP:178).
8. Sets `intent = true` (LC:160-171).

Ordering fact: step 4 mutates schema-middleware state before steps 6 and 7 can fail.
`refuse_parse` (LC:208-229) and `close_statement` (CM:784-796) do not touch the schema middleware.

### 5. Client parameter types (F:829-830)

`client_param_types` is a clone of the client's OID list.
`param_types` is the same list with each `u32` cast to `i32` with `as` (F:830), which wraps for values above `i32::MAX`.
No length relation between this list and the number of placeholders in the SQL is checked in this function.

### 6. Routing (F:834, F:1248-1295)

`self.route(&statement, param_types, &self.context.get_table_resolver())?`.

`get_table_resolver()` builds a new `TableResolver` from the connection's temporary-table resolver in front of the effective resolver (CM:1351-1353, SM:373-378).

`route` decision order:

1. `context.unsafe_disable_mapping()?` true → `Forward(MappingDisabled)` (F:1254-1256). This is a fresh read of connection settings under a read lock (CM:1630-1636), separate from the snapshot captured in step 4.7.
2. `copies_into_table(statement)` is `Some` (only `COPY <table> ... FROM`, EM:103-116) → when `has_encrypted_columns()` (CM:1748-1751), `check_copy_into` must succeed; then `Forward(NothingEncrypted)` (F:1257-1262). `check_copy_into` refuses unresolvable or ambiguous tables, unknown columns, a table with no known columns, an unquoted column named `like`, and any non-native column (F:1400-1455). When `has_encrypted_columns()` is false the COPY is forwarded with no check (F:1258-1261).
3. `!requires_type_check(statement)` (EM:61-72: anything other than Query, Insert, Update, Delete, Merge, Prepare, Explain) → refused with `StatementCouldNotBeTypeChecked` when `has_encrypted_columns() && writes_data(statement)` (F:1266-1271); otherwise `Forward(NotTypeChecked)` (F:1272).
4. `type_check` (F:1274, F:1304-1315, EM:35-47):
   - `Ok(typed)` → `to_encryptable_statement(&typed, param_types)?` (F:1275). `Some` → `Route::Map`; `None` → `Forward(NothingEncrypted)` (F:1276-1280).
   - `Err(err)` → `writes_encrypted = has_encrypted_columns() && writes_data(statement)` (F:1285-1286); the error is classified by `mapping_error` (F:1320-1393); `may_fall_back` returns true only when `!mapping_errors_enabled() && !writes_encrypted && err.is_coverage_gap()` (F:1300-1302). True → `Forward(Fallback(err))` (F:1289); false → `Err(err)` (F:1291).

`to_encryptable_statement` (F:909-958):

- Calls `get_param_columns`, `get_projection_columns`, `get_literal_columns` (F:914-916). Each reads `schema_middleware.encrypt_config()` separately (CM:1762, CM:1770, CM:1785) and each returns `Err(UnknownColumn)` when the type checker reports an EQL term for a column the encrypt config does not contain (COL:189-199).
- Returns `Ok(None)` when no param column is `Some`, no projection column is `Some`, and `!typed_statement.requires_transform()` (F:918-926).
- Otherwise builds `output_params` as `Input(idx)` for each param in enumeration order of `param_columns` (F:939-947) and stores `param_types` as `postgres_param_types` (F:949-955).

`get_literal_columns` pushes only `Some` values (COL:149-152), and `get_column` returns either `Ok(Some)` or `Err` on every path (COL:166-200), so `literal_columns` has one entry per entry of `typed_statement.literals` whenever the call succeeds.

### 7. Route::Map branch (F:835-877)

1. `typed_statement.requires_transform()` (F:839) runs a dry-run of all transformation rules and returns `did_edit()` (TCS:150-155). The dry run's `Result` is discarded (`let _ =`, TCS:152).
2. When true:
   - Records parse duration (F:841-843).
   - `encrypt_literals(session_id, &typed_statement, &statement.literal_columns).await?` (F:845-847, F:631-711):
     - Returns an empty vector when there are no literals (F:638-644).
     - Zips `literal_values()` (which is `&self.literals`, TCS:261-263) with `literal_columns` (F:646-648).
     - Literals whose text parses as an inbound EQL payload are separated out (F:650-660) and skipped during plaintext conversion (F:661-662).
     - `context.encrypt(plaintexts, literal_columns)` (F:666-672) reads the keyset identifier at call time (CM:1658), returns all-`None` without calling the service when every plaintext is `None` (CM:1652-1656), and rejects a result whose length differs from the input (CM:1667-1673).
     - Merges inbound EQL (F:674-675), projects query operands (F:677-686), updates metrics (F:694-708).
   - `transform_statement(&typed_statement, &encrypted_literals)?` (F:849-850, F:718-764):
     - Zips `typed_statement.literals` with the encrypted values, dropping `None` entries (F:743-748).
     - Returns `Ok(None)` when `requires_transform()` is false (F:755-757); this is a second evaluation of the same dry run as F:839.
     - Calls `typed_statement.transform(encrypted_nodes)`; errors become `StatementCouldNotBeTransformed` (F:759-761). `transform` first checks the count and identity of the supplied literal nodes (TCS:168, TCS:277-297), renumbers placeholders, and builds a `ParamPlan` that must cover every input param (TCS:176-179, TCS:253-256, PP:132-146).
   - When `Some(transformed)`:
     - `get_output_param_columns(&transformed.params)?` (F:861-863, CM:1774-1779, COL:104-130) maps each plan output to a column config; this is another separate read of `encrypt_config()`.
     - `statement.output_params` is replaced from the plan (F:864-865, ST:149-178). `zip` at ST:155 truncates to the shorter of plan outputs and columns; `get_output_param_columns` pushes exactly one entry per output (COL:107-127), so the lengths are equal on success. 1-based params are converted with `saturating_sub(1)` (ST:181-183).
     - `statement_text` and `message.query` are replaced with the `Display` rendering of the transformed AST (F:867-868).
3. `STATEMENTS_ENCRYPTED_TOTAL` is incremented for every `Map` route, including one that performed no transform (F:872).
4. `message.parameter_types = rewrite_parse_param_types(&client_param_types, &statement.output_params)` (F:874-875, F:1489-1509):
   - Client declared zero types → outgoing list is empty (F:1490-1492).
   - Otherwise the outgoing list has exactly `output_params.len()` entries (F:1494-1508), independent of how many types the client declared.
   - An output with a column config gets `TEXT` for `JsonAccessor`/`JsonPath` terms and `JSONB` for every other term (F:1497-1502); the client's declared OID for that position is not forwarded.
   - An output without a column config gets the client's OID at `source.primary_input()` or `0` when the client's list is shorter (F:1503-1506, ST:56-62).
5. `context.finish_parse(operation, Some(statement))?` (F:876, LC:177-206):
   - Takes `operations[operation].preparation` and requires `Parse { intent: true }` (LC:186-195). `intent: true` is written only at LC:169.
   - Computes `sensitive = literal_columns.iter().any(Option::is_some)` (LC:196-198).
   - Stores `Arc<Statement>` on the name's entry (LC:199-203, PS:57-59).
   - Sets `literal_sensitive` on the `SettingsProtocol.prepared` entry (LC:204, SP:186-190).

The client's declared OIDs remain in `Statement.postgres_param_types` (F:954) even where the outgoing OID was overwritten.
`Bind` later indexes that list by input param index (BR:91, BR:401-402; called from F:1098-1099).

### 8. Route::Forward branch (F:878-881)

`reason.record()` increments passthrough counters (F:84-101).
`finish_parse(operation, None)` (F:880) takes the preparation, stores no `Statement`, and sets `literal_sensitive = false` (LC:196-204).

After this branch the name's entry exists with `statement == None`, `parse == Some(operation)`, a metrics scope, and a deallocation when one was recorded (PS:7-15).
`message.query` and `message.parameter_types` are untouched on this branch, so the comparison at F:890 is false and the function returns `Ok(None)`.

All four `Forward` reasons (`MappingDisabled`, `NotTypeChecked`, `NothingEncrypted`, `Fallback`) leave the same stored state; the reason is not stored (F:878-881, PS:7-15).

### 9. Parse duration and return (F:884-901)

`record_parse_duration` is first-write-wins per the doc comment at CM:1876 and is skipped when already recorded before encryption (F:885-888).
It is a no-op when the scope id is absent (CM:1870-1872).

The function returns `Ok(Some(Parse))` when either the query bytes or the parameter type list changed (F:890-898), otherwise `Ok(None)` (F:900).
On a `Map` route with no transform and a non-empty client type list, the SQL bytes are the client's original bytes and only the OID list differs.

## Exit paths and the state each leaves

| Exit | Line | Live state at exit |
|---|---|---|
| `begin_parse` error | F:812 | Lock poisoned; metrics scope may exist; preparation may be absent. |
| SQL parse error | F:817 | Name entry exists with `statement None`; prior statement only in journal; `intent false`. |
| deallocation write error | F:820 | Same as above. |
| `prepare_parse` error | F:825 | Same, plus schema-middleware `prepared[name]` may be set (SM:485-488) and `SettingsProtocol.prepared[name]` may be set (SP:168). |
| route error | F:834 | `intent true`; name entry with `statement None`; settings and schema intents set. |
| duration / encrypt / transform / column errors | F:842, F:847, F:850, F:863 | Same as route error. |
| `finish_parse` error | F:876, F:880 | Preparation was taken when the variant matched; otherwise unchanged. |
| late duration error | F:887 | `finish_parse` already succeeded; statement installed; preparation already taken. |
| success | F:898, F:900 | Statement installed (Map) or entry with `statement None` (Forward); journal entry retained until PostgreSQL acknowledges. |

For every `Err` exit the caller runs `refuse_parse` (F:270, LC:208-229), which takes the preparation (any `intent`), removes the name's entry entirely (LC:227), and stores the diagnostic on the operation (LC:228).
For the F:887 exit the preparation is already gone, so `refuse_parse` returns `UnknownOperation` and `intercept` returns `Err` through the `?` at F:270; that exit is reachable only through a poisoned lock (CM:1866-1869).

The journal entry written at SP:145 is retired on PostgreSQL acknowledgement (`preparation_acknowledged`, LC:408-427, SP:326-330) or replayed on PostgreSQL failure (`postgres_failed` → `settings_execution_failed` → `restore_failed_preparation`, LC:473-488, CM:1608-1619, CM:216-239, SP:333-432).
For an unnamed statement whose own Parse failed, the previous unnamed statement is not restored (SP:381-392).

On refusal the caller forwards a substitute `Parse` with the client's statement name, the SQL `SELECT NULL::"__cipherstash_<uuid v4>"`, and an empty type list (F:1731-1743, F:1756-1760).

## Invariants

1. A `Statement` is installed under the name only on `Route::Map` (F:876 passes `Some`, F:880 passes `None`; LC:199-203).
2. `finish_parse` succeeds only for an operation whose `prepare_parse` ran to completion (`intent: true` set only at LC:169; required at LC:191-194).
3. `message.query` is modified only when `transform_statement` returned `Some` (sole write at F:868).
4. `message.parameter_types` is modified only on `Route::Map` (sole write at F:874-875).
5. Every output param that has a column config is declared to PostgreSQL as `JSONB` or `TEXT` whenever the client declared at least one type (F:1490-1502).
6. When the client declared no types, the outgoing type list is empty on every route (F:1490-1492; untouched on Forward).
7. From F:812 until `refuse_parse` or a later close, `prepared_statements.contains(name)` is true (LC:107-112; removal only at LC:227 on the error path).
8. The previous statement stored under the name is removed from live state before the new SQL is examined (LC:103 precedes F:817).
9. `literal_sensitive` for the name equals "the installed statement has at least one literal column" (LC:196-204).
10. A statement that writes data and fails type checking is never routed to `Forward(Fallback)` while `has_encrypted_columns()` is true (F:1285-1291, F:1300-1302).
11. A non-type-checked statement that contains a write is refused while `has_encrypted_columns()` is true (F:1266-1271).
12. `Context::encrypt` never returns a vector whose length differs from the plaintext vector (CM:1652-1656, CM:1667-1673).

## Assumptions and what establishes each

1. **The text Proxy analyses is the text PostgreSQL executes on the no-rewrite path.**
   The analysed text is a lossy UTF-8 decode (F:815); the forwarded bytes are the originals (F:890-900).
   Established by: nothing found. A search for `client_encoding` across `packages/` returned no matches.
2. **sqltk's parse of the text matches PostgreSQL's parse of the same bytes** on the Forward path and on the Map-without-transform path, where PostgreSQL parses the client's bytes itself (F:868 is the only rewrite).
   Established by: nothing found in this crate; PA:90 states "The original wire SQL remains authoritative".
3. **The Parse carries exactly one statement.**
   `parse_statement` returns the first statement and drops the rest in the COPY branch (PA:22-26); behaviour of sqltk `Parser::parse_statement` with trailing tokens is not visible (PA:28-30).
   Established by: nothing found in this crate; relies on PostgreSQL's own handling of a multi-statement Parse.
4. **The mapping-disabled flag read by `route` (F:1254) equals the value captured in the `EffectiveSettings` snapshot (CM:1450).**
   These are two separate lock acquisitions.
   Established by: `frontend_operation` and `backend_operation` both take `&mut self` on the one `CipherStashMiddleware` (MW:54-76), and no `.await` lies between LC:159 and F:834. Whether another clone of the same `Context` exists on another task is not established here (see open questions).
5. **The keyset used to encrypt literals (CM:1658) equals the keyset in the snapshot (CM:1450).**
   Established by: same as 4; no `.await` precedes CM:1658 inside `encrypt_literals` (F:637-668).
6. **The resolver used for type checking and the encrypt config used for column lookup describe the same schema.**
   They are read from different `RwLock`s at different times (SM:373-378 at F:834; SM:387-389 at CM:1762, CM:1770, CM:1785, CM:1778).
   `adopt_latest` writes them one after the other (SM:392-399).
   Established by: nothing found that reads both atomically. A mismatch where the config lacks a column surfaces as `Err(UnknownColumn)` (COL:189-199), not as a `None` column.
7. **The route decided at Parse stays valid for the lifetime of the statement name.**
   `Statement` carries no schema version (ST:89-101), and the Forward reason is not stored (PS:7-15).
   Settings drift is covered at Bind and Execute only when `needs_mapping` is true (SP:213-220, SP:242-249).
   Schema drift between Parse and Bind/Execute: nothing found in this function or in the callees read.
8. **`typed_statement.params` is ordered by placeholder number and dense**, so enumeration index `idx` equals `$n - 1` (F:939-947 uses `enumerate`; COL:74 iterates in the vector's order).
   Established by: `registry.resolved_param_types` (EM:336), not read.
9. **`literal_columns` is positionally aligned with `typed_statement.literals`** (zips at F:646-648, F:677-680, F:743-746, F:1556-1558).
   Established by: COL:139-153 together with COL:166-200 (every success path of `get_column` is `Some`).
10. **`requires_transform()` returns the same answer at F:839, F:755, and F:923.**
    Established by: TCS:150-155 takes `&self` and builds a fresh transformer each call. The dry run's error is discarded (TCS:152), so the answer on a failing dry run is whatever `did_edit()` reports at the point of failure.
11. **PostgreSQL rejects the substitute Parse sent on refusal**, and the backend then replays the journal for this operation.
    Established by: random type name (F:1739-1740, F:1759); PostgreSQL behaviour is external.
12. **Proxy's statement-name key identifies the same object as PostgreSQL's.**
    Proxy keys by the exact client bytes (F:812, PS:34). The comment at F:1757-1758 states PostgreSQL uses `NAMEDATALEN`-byte hash keys for named objects.
    `to_deallocation` lowercases unquoted names with ASCII rules and applies no length limit (F:1463-1470).
    Established by: nothing found.
13. **The number of client-declared types matches the statement's placeholders.**
    No check at F:829-830 or F:1489-1509; missing entries become OID 0 (F:1503-1506) and extra entries are dropped (F:1494).
    Established by: nothing found.
14. **Client OIDs fit in `i32`.**
    `as i32` at F:830 wraps; the wrapped value is stored (F:954) and used at Bind to pick a decode type (BR:91, BR:401-402).
    Established by: nothing found.
15. **`has_encrypted_columns()` reflects the schema the statement will run against.**
    It gates the COPY check (F:1258), the nested-write refusal (F:1266), and the fallback policy (F:1285-1286).
    Established by: schema middleware state (CM:1748-1751); freshness depends on `before_statement` conditions (SM:438-446).
16. **The `operation` id is unique per frontend message and ordered.**
    `journal.insert` overwrites on a repeated id (SP:145), and `deallocate` compares parse operation ids with `<` (PS:118-122).
    Established by: `pg-proto`, source not available.

## Calls

| Callee | Kind | Line | What this function depends on it for |
|---|---|---|---|
| `Context::begin_parse` | internal | F:810-812 | Journal snapshot of the prior statement, fresh entry, `Preparation::Parse{intent:false}`, metrics scope id. |
| `String::from_utf8_lossy` | std | F:815 | Text for analysis; replaces invalid sequences. |
| `SqlParser::parse_statement` | internal over sqltk | F:817 | One AST statement; first-only in COPY branch. |
| `to_deallocation` | internal | F:818 | Classifies DEALLOCATE / DISCARD ALL. |
| `Context::set_statement_deallocation` | internal | F:819-820 | Stores deallocation on the Parse's own name. |
| `Context::prepare_parse` | internal | F:823-825 | Schema adoption, unmodelled-DDL refusal for schema-dependent statements, settings snapshot, keyset-setting validation, `intent = true`. |
| `Context::get_table_resolver` | internal | F:834 | Resolver for type checking. |
| `Frontend::route` | internal | F:834 | The Map / Forward / refuse decision and the `Statement` skeleton. |
| `eql_mapper::type_check`, `requires_type_check`, `writes_data`, `copies_into_table` | external, source available | via F:1257-1274 | Classification of the statement and EQL typing of params, literals, projection. |
| `TypeCheckedStatement::requires_transform` | external, source available | F:839 | Whether the SQL must be rewritten. |
| `Context::record_parse_duration` | internal | F:841-842, F:886-887 | Metrics only. |
| `Frontend::encrypt_literals` | internal | F:845-847 | Encrypted values positionally aligned with literals. |
| `Context::encrypt` → `EncryptionService::encrypt` | internal → ZeroKMS | via F:666-669 | Ciphertexts under the connection's current keyset; length check at CM:1667. |
| `Frontend::transform_statement` → `TypeCheckedStatement::transform` | internal → external, source available | F:849-850 | Rewritten AST and `ParamPlan` covering all inputs. |
| `Context::get_output_param_columns` | internal | F:861-863 | Column config per rewritten placeholder. |
| `output_params_from_plan` | internal | F:864-865 | 0-based output param descriptors. |
| `rewrite_parse_param_types` | internal | F:874-875 | Outgoing OID list. |
| `Context::finish_parse` | internal | F:876, F:880 | Installs `Statement`, sets literal sensitivity, clears the preparation. |
| `Forward::record` | internal | F:879 | Counters and a warning log only. |
| `counter!`, `debug!` | external | F:852, F:872, F:893 | Metrics and logs; the transformed statement, param plan, and rewritten Parse message are logged at debug level (F:852-856, F:893-896). |

## Shared state touched

- `ConnectionProtocolState.prepared_statements[name]` — removed and recreated (LC:103-112), deallocation (CM:805-809), statement (LC:199-203).
- `ConnectionProtocolState.operations[operation].preparation` — LC:113, LC:169, LC:189.
- `ConnectionProtocolState.statement_metrics` — CM:550-559, LC:93, LC:104-106, LC:154-157, CM:1877-1885, F:697-704.
- `SettingsProtocol.journal[operation]` — SP:145.
- `SettingsProtocol.prepared[name]` — SP:168, SP:186-190.
- `ConnectionSettings` — read only (CM:1450, CM:1635, CM:1643).
- `SchemaMiddleware` — `prepared[name]` written (SM:485-488); base, resolver, encrypt config replaced by `adopt_latest` (SM:392-402) through `before_statement` or `publication_succeeded`; resolver and encrypt config read.
- `CommittedSchemaStore` — read by `adopt_latest` (SM:393); reload triggered at CM:1426.
- ZeroKMS cipher cache — through `EncryptionService::encrypt` (CM:1661-1664).
- Prometheus counters — PA:33, F:872, F:84-101, F:671, F:706-708, F:1331, F:1348, F:1360, F:1372.

## Open questions

- unclear; need to inspect sqltk 0.11.0 `Parser::parse_statement` to learn what it does with tokens after the first statement and with an empty query string.
- unclear; need to inspect `pg_proto::Parse` (crate `pg-proto = "0.12.0"`, source not in the repository) to learn whether `query` and `statement` include a trailing NUL and how `parameter_types` length is bounded.
- unclear; need to inspect `pg-proto`'s `Intermediary` to learn whether `IntermediaryMiddlewareFactory::create` (MW:31-33) is called once per connection, whether frontend and backend operations are serialised, and whether a `frontend_operation` future can be dropped mid-await (between LC:113 and `finish_parse`).
- unclear; need to inspect `SchemaMiddleware::execution_failed`, `protocol_boundary`, and `discard_skipped_executions` to learn whether `prepared[name]` written at SM:485-488 is rolled back when the Parse is refused locally or rejected by PostgreSQL.
- unclear; need to inspect `bind_handler` (F:1003-1080), `Context::begin_bind` (LC:231-), and `SchemaMiddleware::bind`/`execute` to learn whether anything re-validates a stored route (either `Some(Statement)` or `None`) against a schema that changed after Parse.
- unclear; need to inspect `eql_mapper` `registry.resolved_param_types` to confirm the ordering and density of `TypeCheckedStatement.params`.
- unclear; need to inspect `DryRunnable::did_edit` and the transformation rules to learn what `requires_transform` returns when the dry run fails (TCS:152).
- unclear; need to inspect PostgreSQL's handling of protocol-level statement names longer than 63 bytes to confirm or refute the key-identity assumption (assumption 12).
- unclear; need to inspect `error_to_response` (`postgresql/error_handler.rs:37`) for what parts of the client SQL or schema names are echoed in the refusal diagnostic.
- unclear; need to inspect `Backend` handling of `ParseComplete` and `ErrorResponse` to confirm that `preparation_acknowledged` / `postgres_failed` are invoked with this Parse's operation id, including for the substitute Parse sent on refusal.
- unclear; need to inspect `inbound_eql::parse` and `merge_inbound_eql` (F:653-675) for what is accepted when a literal in the SQL is itself an EQL payload.
