# Frontend::query_handler

- File: `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`
- Lines: L394-L607
- Signature: `async fn query_handler(&mut self, operation: OperationId, query: bytes::Bytes, session_id: SessionId) -> Result<Option<FrontendMessage>, Error>`
- Format note: `ANALYSIS_FORMAT.md` was not present on disk (the `{baseDir}` placeholder did not resolve), so this file uses the section order purpose, reachability, inputs, outputs, block-by-block, callees, invariants, assumptions, shared state, open questions.

Abbreviations used for citations:

- `fe` = `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`
- `ctx` = `packages/cipherstash-proxy/src/postgresql/context/mod.rs`
- `lc` = `packages/cipherstash-proxy/src/postgresql/context/lifecycle.rs`
- `sp` = `packages/cipherstash-proxy/src/postgresql/context/settings_protocol.rs`
- `st` = `packages/cipherstash-proxy/src/postgresql/context/settings.rs`
- `sm` = `packages/cipherstash-proxy/src/proxy/schema/middleware.rs`
- `ps` = `packages/cipherstash-proxy/src/postgresql/parser.rs`
- `em` = `packages/eql-mapper/src/eql_mapper.rs`
- `tcs` = `packages/eql-mapper/src/type_checked_statement.rs`
- `cm` = `packages/cipherstash-proxy/src/postgresql/column_mapper.rs`
- `be` = `packages/cipherstash-proxy/src/postgresql/middleware/backend.rs`
- `drv` = `packages/cipherstash-proxy/src/postgresql/driver.rs`

## 1. Purpose

Handles one simple-protocol `Query` message.
It parses the SQL text into zero or more statements, runs four whole-batch refusal checks, decides a `Route` for every statement, encrypts literals of mapped statements, registers the batch's execution, schema and settings effects on the connection `Context`, and returns either a rewritten `Query` message or `None` (forward the client's bytes unchanged).

## 2. Reachability and caller contract

The only caller is `Frontend::intercept` (fe L217-L235).
`intercept` reaches the call only when all of the following hold:

- the message is not `Terminate` (fe L176-L178);
- `failed_extended_batch` is false and `context.discarding_extended_messages()` is false (fe L179-L187);
- the configuration-level `context.mapping_disabled()` is false (fe L189-L211); when it is true, `Query` goes to `context.unmapped_query` and is forwarded without reaching `query_handler` (fe L191, L210).

`intercept` creates the statement metrics scope with `start_metrics_scope` and passes its id as `session_id` (fe L218-L219).

The caller maps the three results as follows:

- `Ok(Some(mapped))` replaces the outbound message (fe L220).
- `Ok(None)` leaves `outbound_message` as the clone of the client's original message made at fe L213 (fe L222).
- `Err(err)` is converted with `error_to_response`, passed to `context.refuse_query`, and the outbound message becomes `simple_error_query(&response)`, a `DO '...RAISE EXCEPTION...'` statement sent upstream in place of the client's SQL (fe L223-L233, L1703-L1729).

Actor: the authenticated client application supplies `query` in full.
`operation` is assigned by the pg-proto pipeline (not visible in this file).

## 3. Inputs

| Input | Source | Controlled by |
|---|---|---|
| `query: bytes::Bytes` | body of the `Query` message | client |
| `operation: OperationId` | pg-proto | proxy pipeline |
| `session_id: SessionId` | `start_metrics_scope` (ctx L550-L559) | proxy |
| confirmed `ConnectionSettings` (keyset, unsafe_disable_mapping) | `protocol_state.settings` | earlier client `SET` commands confirmed by PostgreSQL |
| effective schema resolver, temporary tables, encrypt config, `in_flight_ddl`, `unmodelled` flags | `SchemaMiddleware` | catalog snapshot plus this connection's confirmed DDL |
| `default_keyset_id`, `mapping_errors_enabled` | `TandemConfig` | operator |

## 4. Outputs and side effects

Return value: see section 2.

State written on the success path, in order:

1. Statement metrics scope metadata `protocol = Simple` (fe L403-L405).
2. `STATEMENTS_TOTAL` counter, incremented by the statement count inside `parse_statements` (ps L82).
3. `operations[operation].deallocations` extended with every `DEALLOCATE` / `DISCARD ALL` in the batch (fe L421-L427, ctx L829-L846).
4. Schema middleware: possible publication reload plus `adopt_latest` (fe L428, ctx L1421-L1433, sm L438-L446).
5. Parse duration, encrypt duration, `encrypted`, `encrypted_values_count` on the metrics scope (fe L495-L497, L532-L534, L544-L547; `encrypt_literals` fe L697-L704).
6. Prometheus counters for each route (fe L528, L537 via `Forward::record` fe L84-L101).
7. Metadata `statement_type`, `multi_statement`, `query_fingerprint` (fe L550-L566).
8. `register_query` (fe L568-L569, lc L370-L382): installs the unnamed portal, sets the operation's `ExecuteContext` (until readiness), queues one schema execution per forwarded statement plus a `ReadyBoundary`, and installs a `Pending` settings entry for the operation.

State written before an `Err` return: items 1 to 3 are written before any refusal other than a parse error or a `validate_setting` error; item 4 before any refusal from L429 onward; items 5 and 6 are written incrementally inside the loop (L480-L541) and remain if a later statement's `encrypt_literals` or `transform_statement` fails.
Item 8 is never written on an `Err` path, because `register_query` is the last fallible state call before the return (fe L568-L569) and every earlier `?` exits before it.

## 5. Block-by-block

### 5.1 Setup and parse (L400-L411)

- L400 starts a wall-clock timer used only for logging thresholds at L588 and L598.
- L403-L405 sets `protocol`. `update_statement_metadata` silently does nothing if the scope id is absent (ctx L1870-L1873) and fails only when the `RwLock` is poisoned (ctx L1866-L1869).
- L410 converts the bytes with `String::from_utf8_lossy(&query).into_owned()`.
  Invalid UTF-8 sequences become U+FFFD in `query_text`.
  The original `query` bytes are not used again in this function; on `Ok(None)` the caller forwards the original bytes (fe L213, L222).
  The text that is analysed and the bytes that are forwarded therefore differ whenever the input is not valid UTF-8.
  Whether `query` still carries the protocol's trailing NUL is not determinable from this file.
- L411 `SqlParser::parse_statements(&query_text)?`.
  The parser tokenises with the PostgreSQL dialect (ps L94-L96), rejects `SET [SESSION|LOCAL] ALL` (ps L108-L113), rewrites `RESET x` to `SET x TO DEFAULT` with an empty-span `DEFAULT` token (ps L114-L138), rewrites `RESET ROLE` / `TIME ZONE` / `SESSION AUTHORIZATION` / `TRANSACTION ISOLATION LEVEL` to `SET "__cipherstash_reset" TO '<original tokens>'` (ps L139-L168), and, when any statement is `COPY ... FROM ... STDIN`, splits on every `;` token and parses each piece on its own (ps L74-L75, L220-L236).
  A parse error returns `Err` here; nothing but item 1 and no counter has been written.
  The result may be an empty vector; no line in `query_handler` rejects an empty batch.

### 5.2 Settings validation (L412-L420)

- L412-L414 calls `context.validate_setting` for each statement.
  It calls `SettingChange::parse` (lc L523, st L33-L112), which returns `Err` for an unknown `CIPHERSTASH.*` name (st L82-L84), a malformed mapping value (st L72-L77), a malformed keyset value (st L100-L102) or an unparsable keyset UUID (st L105-L106).
  It additionally returns `EncryptError::UnexpectedSetKeyset` when the change is `Keyset(..)` and a default keyset is configured (lc L522-L528).
  `ResetKeyset` is not matched by that condition (lc L524).
- L415-L420 calls `settings_simple_query_requires_fail_closed`.
  It returns `Ok(true)` when the protocol-state lock is poisoned (ctx L1535-L1537).
  Otherwise it projects each statement over a clone of the *confirmed* settings (sp L47-L61) and returns `true` when a statement for which `eql_mapper::requires_schema` is true follows a statement whose command `may_change_effective` (ctx L1539-L1546).
  `may_change_effective` is `false` for `Other`, `Begin`, `Savepoint`, `Release` (st L190-L195); for the rest it compares effective keyset and mapping flag before and after applying the command to the projected settings, with a second comparison for `Commit` treated as `ROLLBACK` (st L196-L208).
  A `true` result returns `MappingError::DependentStatementAfterSettings`.
  The statements this check guards are exactly those for which `requires_schema` is true: `Query`, `Insert`, `Update`, `Delete`, `Merge`, `Prepare`, `Explain`, and `COPY <table> FROM` (em L61-L72, L103-L122).

### 5.3 Deallocations (L421-L427)

`to_deallocation` (fe L1458-L1478) maps `DEALLOCATE ALL` and `DISCARD ALL` to `AllStatements`, an unquoted `DEALLOCATE name` to the ASCII-lowercased name, and a quoted name to itself.
`set_deallocations` appends them to `operations[operation].deallocations` and is a no-op for an empty list (ctx L834-L845).
This write happens before the schema checks, routing and encryption, each of which can still return `Err`.
On an `Err` return the caller's `refuse_query` (lc L384-L397) calls `set_operation_error` and `set_execute` on the same `operations` entry; neither clears `deallocations` (ctx L1045-L1058, L640-L655).
The queue is consumed by `Backend` on a `CommandComplete` whose tag is a deallocation (be L163-L170) and the whole operation entry is removed at the readiness boundary (ctx L984-L994).

### 5.4 Schema preparation and DDL fail-closed (L428-L438)

- L428 `prepare_schema_for_statement().await?`.
  If the connection is idle, has no local changes, and the shared store has a pending publication, it awaits `reload_schema` and returns `ConfigError::SchemaCouldNotBeLoaded` on failure (ctx L1422-L1430, sm L420-L424).
  It then calls `before_statement`, which adopts the latest committed snapshot only when no transaction is active, there are no local changes, no publication is pending, and `in_flight_ddl == 0` (sm L438-L446).
  `adopt_latest` replaces base schema, encrypt config and resolver, clears savepoints, and clears `unmodelled` but not `local_unmodelled` (sm L392-L402).
- L429-L434 `simple_query_requires_fail_closed`: returns `true` when a `requires_schema` statement follows a statement for which `ddl_may_change_encryption` is true (sm L541-L552).
  `ddl_may_change_encryption` (sm L555-L610) is true for: `CREATE TABLE` whose name is in the encrypt config, or with `AS` query / `LIKE` / `CLONE` / `INHERITS`, or with a column whose kind is `Eql`; `ALTER TABLE` adding an `Eql` column, dropping or renaming a configured column, renaming a configured table, or any operation not classified encryption-neutral; `DROP TABLE` of a configured table; any `CREATE VIEW` or `DROP VIEW`.
  Every other statement kind yields `false` (sm L608).
  The classification reads `self.encrypt_config()` at call time (sm L558, L573, L600).
- L435-L438: only when some statement `requires_schema`, the function awaits `wait_for_schema_execution` and then `ensure_schema_modelled`.
  `wait_for_ddl` loops on a `Notify` until `in_flight_ddl == 0` and has no timeout (sm L675-L683).
  `ensure_schema_modelled` returns `MappingError::UnmodelledDdl` when `unmodelled` or `local_unmodelled` is set (ctx L1408-L1413, sm L433-L435).

Ordering fact: the fail-closed evaluation at L429-L434 runs before the wait at L436.
`batch_resolvers` at L450 evaluates `ddl_may_change_encryption` again (sm L880) after the wait.
Both read `encrypt_config`, which `Backend`-reported `execution_succeeded` mutates through `apply_ddl` (sm L788-L790, L928-L936).
The two evaluations therefore read the same state only if no DDL outcome lands between L431 and L450.

### 5.5 Routing (L439-L452)

`batch_table_resolvers` returns one resolver per statement (sm L872-L916).
The resolver for statement *i* is the effective resolver (temporary tables in front of the effective schema, sm L373-L378) with the changes of statements `0..i` applied to scratch copies, where a change is applied only if the statement is not `ddl_may_change_encryption` (sm L880-L882), `split_change` classifies it (sm L883-L886), and a catalog change is applied only when `models_ddl` is true (sm L899-L905).
Unmodelled or non-modelled catalog changes are skipped without marking anything (sm L906).
The scratch copies are snapshots, so connection state is not changed (sm L227-L231, L920-L925).
`zip` at L450 pairs statements and resolvers; both sequences come from `parsed_statements`, and `batch_resolvers` maps one-to-one (sm L875-L915).

`route` is called with empty `param_types` (L451).
Its decision order (fe L1248-L1295):

1. `unsafe_disable_mapping()` true (confirmed settings, ctx L1630-L1636) -> `Forward(MappingDisabled)`.
2. `COPY <table> FROM` -> if `has_encrypted_columns()`, `check_copy_into` must pass (fe L1400-L1455), else no check; then `Forward(NothingEncrypted)`.
3. Not `requires_type_check` -> `Err(StatementCouldNotBeTypeChecked)` if `has_encrypted_columns() && writes_data(statement)`, else `Forward(NotTypeChecked)`.
4. Type check OK -> `to_encryptable_statement` -> `Map` or `Forward(NothingEncrypted)`.
5. Type check error -> `mapping_error`, then `Forward(Fallback(err))` only when `!mapping_errors_enabled() && !writes_encrypted && err.is_coverage_gap()` (fe L1300-L1302; `is_coverage_gap` is `StatementCouldNotBeTypeChecked | Internal`, `error.rs` L67-L74), else `Err`.

`collect::<Result<Vec<_>, _>>()?` at L452 stops at the first `Err`; statements after it are not routed.
Routing runs for every statement before any encryption (comment L446-L447, code L448-L452 precedes L480).

`to_encryptable_statement` (fe L909-L958) resolves param, projection and literal columns through `ColumnMapper` built from `schema_middleware.encrypt_config()` (ctx L1758-L1787), which is the connection's effective encrypt config, not a per-statement batch view.
The batch resolver supplies table shapes; the encrypt config supplies column configuration; they are two separate sources.
`get_column` returns `Ok(Some)` or `Err(UnknownColumn)` and has no `Ok(None)` path (cm L166-L200), so `literal_columns` has exactly one entry per `typed_statement.literals` entry when it returns `Ok` (cm L139-L153).

### 5.6 Whole-batch rules (L454-L474)

- L456-L462: if any route is `Map` and any parsed statement is `ast::Statement::Copy { .. }` (either direction), return `CopyWithEncryptedStatement`.
  The condition uses `is_map`, not the later `encrypted` flag, so it also fires for a batch that would be forwarded unchanged.
- L467-L474: if any route is `Map` and any route is `Forward(Fallback(_))`, return the first fallback's carried error.
  `swap_remove` reorders `routes`, which is harmless because the function returns on the next line (L469-L472).
  When no route is `Map`, fallbacks remain and the batch is forwarded as written (`encrypted` stays false, L596-L605).

### 5.7 Per-statement loop (L476-L541)

Initial values: `portal = Portal::passthrough(Some(session_id))`, `encrypted = false` (L476-L477).

`Route::Map` (L482-L535):

- If `typed_statement.requires_transform()` (a dry-run transform, tcs L150-L155): record parse duration once (L494-L498), `encrypt_literals(...).await?` (L500-L506), `transform_statement(...)?` (L508-L509).
  `transform_statement` returns `None` only when `!requires_transform()` (fe L755-L757), which L492 already excluded, so inside this block the `Some` arm is the only non-error outcome and `encrypted` and `transformed` both become true (L518-L520).
- If not transformed, the original parsed AST (`typed_statement.statement.clone()`) is pushed (L524-L526).
- `STATEMENTS_ENCRYPTED_TOTAL` is incremented for every `Map` route, transformed or not (L528).
- L531 overwrites `portal` with `Portal::encrypted(Arc::new(encryptable), Some(session_id))`.

`Route::Forward` (L536-L539): record the reason, push the parsed statement clone.
It does not touch `portal`.

Consequences that follow from L476, L531 and L536-L539:

- Exactly one statement is pushed to `forwarded_statements` per iteration (L518 xor L525, or L538), so `forwarded_statements.len() == parsed_statements.len()` after the loop.
- `portal` after the loop is the `Statement` of the *last* `Map` route in the batch, or passthrough when there is none.
  A `Forward` statement that follows a `Map` statement leaves the encrypted portal in place.
- That single portal is captured into the operation's `ExecuteContext` (ctx L669-L671) and is what `Backend` reads for every `DataRow` and every `RowDescription` of the operation (ctx L1290-L1307; be L478-L502, L662-L697, L733-L752).

An `Err` from `encrypt_literals` or `transform_statement` on statement *k* leaves counters and metrics for statements `0..k` recorded and encryption requests for them already sent.

### 5.8 Metadata, registration, rewrite (L543-L607)

- L550-L557: `statement_type` is derived only for a single-statement batch; an empty or multi-statement batch is `Other`.
- L564-L566: fingerprint is a keyed Blake3 hash of `query_text`, truncated to 4 bytes (`statement_metadata.rs` L111-L120).
- L568-L569 `register_query(operation, session_id, portal, &forwarded_statements)`.
  It is given the *forwarded* (possibly transformed) ASTs, not `parsed_statements`.
  `add_portal` installs the portal under the empty name, replacing any existing unnamed portal and unnamed suspended execution (lc L377, ctx L188-L214).
  `set_simple_query_execute_until_ready` snapshots that portal into the operation (ctx L657-L673).
  `SchemaMiddleware::simple_query` sets `transaction_active`, pushes one `Execute(intent)` per statement, increments `in_flight_ddl` per schema-changing intent, then pushes `ReadyBoundary` (sm L526-L537).
  `SettingsProtocol::simple_query` re-derives one `SettingsCommand` per statement and stores them as `Pending { until_ready: true }` (sp L262-L286).
  `SettingsCommand::from_statement` is fallible (st L170-L187); it was already evaluated on `parsed_statements` at L412-L417.
- L571-L595: when `encrypted`, every forwarded statement is rendered with `SqlParser::statement_sql` and joined with `";"`.
  `statement_sql` renders the RESET sentinels back to `RESET ...` and otherwise uses the AST `Display` (ps L40-L69).
  All statements, including those routed `Forward`, are re-rendered from the AST; none of the client's original text is reused.
  The rewritten SQL and the message are logged at debug level (L580-L587).
- L596-L605: when not `encrypted`, return `Ok(None)`.
  This includes a batch with `Map` routes none of which required a transform (encrypted projection only): the portal is `Encrypted`, the bytes are the client's.

## 6. Callees

### 6.1 Internal, source read

| Callee | What this function depends on it to establish | Paths that do not establish it |
|---|---|---|
| `SqlParser::parse_statements` (ps L72-L85) | The returned ASTs describe the statements PostgreSQL will execute from the same text, one AST per executed statement, in order. | Nothing in the parser compares against PostgreSQL's own grammar. The `COPY FROM STDIN` branch splits on every `;` token (ps L224) and its own comment states a multi-statement body fails to parse (ps L215-L219). |
| `Context::validate_setting` (lc L521-L530) | Malformed or unknown `CIPHERSTASH.*` settings and keyset changes under a default keyset are refused before anything is registered. | `ResetKeyset`, `ResetAll`, `DiscardAll` are not matched by the keyset condition (lc L522-L525). |
| `Context::settings_simple_query_requires_fail_closed` (ctx L1531-L1548) | No `requires_schema` statement in this batch is routed or encrypted under settings that an earlier statement of the same batch may change. | Projection starts from confirmed settings only (sp L51); it does not include commands of earlier operations still in flight. Statements for which `requires_schema` is false are never a reason to fail closed (ctx L1542). |
| `Context::set_deallocations` (ctx L829-L846) | Deallocations are queued on the operation. | Not undone on a later `Err` in this function. |
| `Context::prepare_schema_for_statement` (ctx L1421-L1433) | An idle connection routes against the latest committed snapshot. | Adoption is skipped when a transaction is active, local changes exist, a publication is pending, or DDL is in flight (sm L439-L442). |
| `Context::simple_query_requires_fail_closed` (ctx L1376-L1382, sm L541-L552) | No `requires_schema` statement follows encryption-changing DDL in the same batch. | DDL kinds outside the `match` at sm L556-L609 return `false`. Evaluated before the wait at L436. |
| `Context::wait_for_schema_execution` (ctx L1390-L1392, sm L675-L683) | All earlier schema-changing executions have an outcome before routing. | Called only if some statement `requires_schema` (fe L435). No timeout. |
| `Context::ensure_schema_modelled` (ctx L1408-L1413) | No confirmed unmodelled DDL is outstanding when schema-dependent statements are routed. | Called only inside the same `if` at L435. |
| `Context::batch_table_resolvers` (ctx L1739-L1744, sm L872-L916) | One resolver per statement, reflecting earlier encryption-neutral DDL of the batch. | Catalog DDL that is not `models_ddl` and `SchemaChange::Unmodelled` are skipped silently (sm L906); the later statement then type checks against a view without that change. |
| `Frontend::route` (fe L1248-L1295) | Every statement has a named decision; a refusal is `Err`. | Rule 1 returns `Forward(MappingDisabled)` before any schema check (fe L1254-L1256). Rule 2 skips `check_copy_into` when `has_encrypted_columns()` is false (fe L1258). |
| `Frontend::encrypt_literals` (fe L631-L711) | One `Option<EqlOutput>` per literal, aligned with `typed_statement.literals`. | Returns an empty vector when there are no literals (fe L638-L644). Alignment rests on `Context::encrypt`'s length check (ctx L1667-L1673) and on `literal_columns` length (cm L139-L153). |
| `Frontend::transform_statement` (fe L718-L764) | The rewritten AST contains the ciphertext for every non-null literal. | Relies on `TypeCheckedStatement::transform`'s count check (tcs L277-L297). |
| `Context::register_query` (lc L370-L382) | Portal, execute context, schema executions and settings commands for the batch are queued for `Backend` to correlate. | Four sequential steps with `?` between them; a failure at `execute_simple_settings_statements` (lc L380) leaves the portal, execute context and schema executions already installed. |
| `SqlParser::statement_sql` (ps L40-L69) | The rendered text is semantically the statement the client sent, with RESET forms restored. | Falls through to AST `Display` for everything else (ps L45, L67). |
| `Context::update_statement_metadata`, `record_parse_duration` (ctx L1877-L1917) | Diagnostics only. | No-op when the scope id is missing (ctx L1870-L1873). |

### 6.2 External, source available in workspace (eql-mapper)

| Callee | Depends on | Notes |
|---|---|---|
| `eql_mapper::requires_schema`, `requires_type_check`, `copies_into_table`, `writes_data` (em L61-L122) | Classification of statement kinds. | `requires_type_check` is a fixed list of seven variants (em L62-L71). `writes_data` visits nested statements (em L76-L96). |
| `eql_mapper::type_check` (em L35-L47) | An `Ok` means every referenced table and column resolved and every literal bound for an EQL column is listed in `literals`. | Internals of inference not read for this analysis. |
| `TypeCheckedStatement::requires_transform`, `transform`, `literal_values` (tcs L150-L182, L261-L263) | `transform` fails unless the number of supplied encrypted literals equals the number of non-null literals and every key names a known literal node (tcs L277-L297). | Transformation rules not read. |

### 6.3 External, treated as black box

| Callee | Sent | Assumed | Outcomes not excluded |
|---|---|---|---|
| `EncryptionService::encrypt` via `Context::encrypt` (ctx L1647-L1675) | Plaintexts of literals, column configs, the confirmed keyset identifier. | Returns one output per input in the same order, each bound to the given column and keyset. | Error; a result of the right length whose entries do not correspond positionally (only the length is checked, ctx L1667); arbitrary latency while `&mut self` is held across `.await` (fe L500-L506). |
| `EncryptionService::decrypt_inbound_eql` via `merge_inbound_eql` (fe L1185-L1197) | Client-supplied EQL ciphertext found in a literal. | Authenticates the ciphertext under the confirmed keyset. | Error; plaintext count differing from ciphertext count (the later `zip` at fe L1221 truncates). |
| `reload_schema` -> reload channel (ctx L1695-L1715) | A `ReloadCommand::DatabaseSchema`. | Responds `Ok(true)` only after a new snapshot is published. | Channel closed; no response (awaits `receiver` with no timeout, ctx L1712). |
| sqltk parser (`Parser::parse_statements`, AST `Display`) | Client SQL tokens. | Grammar and rendering agree with PostgreSQL for everything it accepts. | Accepting text PostgreSQL parses differently; rendering an AST to different semantics. |
| `metrics` macros, `tracing` | Counters, and at debug level the rewritten SQL (fe L511-L514, L580-L587). | No effect on control flow. | - |

## 7. Invariants

1. No statement's literals are encrypted before every statement in the batch has a route (fe L448-L452 precedes L480).
2. A batch that contains both a `Map` route and a `Forward(Fallback)` route returns `Err` (fe L467-L474).
3. A batch that contains a `Map` route and any `COPY` statement returns `Err` (fe L456-L462).
4. `forwarded_statements.len() == parsed_statements.len()` at L568 (one push per loop iteration: fe L518 / L525 / L538).
5. A rewritten `Query` is returned only when at least one statement was transformed (fe L519, L571); otherwise the client's bytes are forwarded (fe L605, caller fe L222).
6. `register_query` is not called on any `Err` path (every `?` and `return Err` precedes fe L568 except the `?` on L569 itself).
7. A `requires_schema` statement is never routed in a batch where an earlier statement may change effective settings (fe L415-L420, ctx L1542-L1543) or is encryption-changing DDL as classified at check time (fe L429-L434, sm L545-L548).
8. When the batch contains a `requires_schema` statement, routing happens only with `in_flight_ddl == 0` observed at L436 and with neither unmodelled flag set at L437 (fe L435-L438).
9. Within the `requires_transform()` branch, a statement is pushed only in its transformed form (fe L492, L508-L521; `transform_statement` `None` path at fe L755-L757 is excluded by L492).
10. Exactly one portal is registered per `Query` operation, under the empty name (lc L377-L378).

## 8. Assumptions

| # | Assumption | Established by |
|---|---|---|
| A1 | sqltk splits and classifies the SQL text into the same statements, in the same order, that PostgreSQL executes from the forwarded bytes. | nothing found (ps L72-L85 parses; no comparison exists). |
| A2 | `query_text` equals the forwarded bytes. | nothing found for non-UTF-8 input: `from_utf8_lossy` at fe L410 alters the analysed text while fe L213/L222 forward the original bytes. |
| A3 | Re-rendering every statement via AST `Display` (fe L572-L576) preserves the meaning of statements routed `Forward`. | Partly: `COPY` is excluded by fe L456-L462 and RESET forms by ps L47-L66. Nothing found for other statement kinds. |
| A4 | Confirmed settings read by `route` (fe L1254) and `Context::encrypt` (ctx L1658) are the settings that will be effective when PostgreSQL executes this batch. | Within the batch: fe L415-L420. Across earlier operations: `drv` L214-L267 drains backend outcomes after a forwarded `Query`/`Sync` while settings or DDL are in flight; that loop exits early at drv L241-L245. Not established inside `query_handler`. |
| A5 | A statement for which `requires_schema` is false needs no protection from a preceding settings change or encryption-changing DDL in the same batch. | `route` rule 3 refuses a nested write when encrypted columns exist (fe L1266-L1271). For other non-type-checked kinds: nothing found. |
| A6 | `ddl_may_change_encryption` gives the same answer at fe L431 (before the wait) and at sm L880 (after the wait, via fe L450). | nothing found in this function; holds only if no DDL outcome is applied between the two calls. |
| A7 | `wait_for_schema_execution` at fe L436 is woken. | `Backend` calls `execution_succeeded` / `execution_failed` / `ready_for_query` (sm L811-L812, L943-L944, L977-L981). The driver comment at drv L226-L228 states the intermediary is single-task and that a frontend mapping call can wait for an outcome the task has not read; drv L220-L267 is the stated mitigation. No timeout exists at fe L436 or sm L675-L683. |
| A8 | The single registered portal (last `Map` statement's projection columns, fe L531) is the right decryption description for the rows of every statement in the batch. | nothing found in `query_handler`; `Backend` reads one portal per operation (ctx L1290-L1307, be L478-L502, L733-L752). |
| A9 | PostgreSQL emits exactly one completion or error per statement in `forwarded_statements`, in order, so the schema execution queue (sm L526-L537) and settings command queue (sp L269-L283) stay aligned. | nothing found here; realignment at readiness is done by `discard_skipped_executions` (sm L963-L982) and `ready_for_query` (ctx L931-L1015). Depends on A1. |
| A10 | The schema-intent and settings-command classification of the *transformed* ASTs passed at fe L569 equals that of the parsed ASTs checked at fe L412-L434. | Forward routes pass the parsed clone unchanged (fe L538); for Map routes it rests on the mapper only rewriting DML (em L61-L72). Not checked at fe L568. |
| A11 | `literal_columns` is positionally aligned with `typed_statement.literals`. | cm L139-L153 with cm L166-L200 (no `Ok(None)` path). The `if col.is_some()` filter at cm L150 would break alignment if `get_column` gained an `Ok(None)` path. |
| A12 | The encrypt config used to build `Statement` (ctx L1758-L1787) describes the same tables as the batch resolver used for type checking (sm L872-L916). | Batch resolvers skip encryption-changing DDL (sm L880-L882) and fe L429-L434 refuses dependent statements; subject to A6. |
| A13 | Deallocations queued at fe L421-L427 are harmless when the batch is later refused. | `refuse_query` does not clear them (lc L384-L397); the entry is removed at readiness (ctx L984-L994). The replacement `DO` statement produces no deallocation `CommandComplete` only if PostgreSQL raises as written (fe L1727-L1728). |
| A14 | `has_encrypted_columns()` is accurate, since rules 2 and 3 of `route` skip their checks when it is false (fe L1258, L1266). | ctx L1748-L1751: encrypt config non-empty or base schema has encrypted columns; both come from the adopted snapshot. |
| A15 | Installing the unnamed portal for a simple `Query` (lc L377) does not disturb extended-protocol state the client still relies on. | `install_portal` removes any existing unnamed portal and unnamed suspended execution (ctx L194-L204); PostgreSQL also destroys the unnamed portal on a simple Query (protocol behaviour, not checked in code). |
| A16 | `EncryptionService::encrypt` returns outputs in input order. | Length only: ctx L1667-L1673. Order: nothing found. |

## 9. Shared state touched

- `ConnectionProtocolState.statement_metrics[session_id]` (metadata, phase timing): fe L403, L495, L532, L545, L558, L564; fe L697-L704.
- `ConnectionProtocolState.operations[operation].deallocations`: fe L421-L427.
- `ConnectionProtocolState.operations[operation].execute`, `.portals[""]`, `.portal_operations[""]`, `.suspended_executions[""]`: via `register_query` (lc L377-L378).
- `ConnectionProtocolState.settings` (read): ctx L1535-L1540, L1630-L1636, L1638-L1644.
- `SettingsProtocol.pending[operation]` and boundary: lc L380, sp L276-L284.
- `SchemaMiddleware`: `base`, `base_encrypt_config`, `encrypt_config`, `resolver`, `savepoints`, `unmodelled`, `dirty` (via publication / `adopt_latest`, fe L428); `transaction_active`, `executions`, `in_flight_ddl` (via `simple_query`, lc L379); read of `temporary`, `resolver`, `encrypt_config`, `in_flight_ddl`, `unmodelled`, `local_unmodelled`.
- `CommittedSchemaStore` (read `publication_pending`, `load`; write through reload): fe L428.
- ZeroKMS cipher cache (through `EncryptionService`): fe L500-L506.
- Prometheus counters and histograms: ps L82; fe L84-L101, L528; fe L671, L706-L708.

## 10. Open questions

- Unclear whether `query` includes the protocol's trailing NUL byte when it reaches fe L410; need to inspect pg-proto's `FrontendMessage::Query` decoding.
- Unclear whether `in_flight_ddl` can be non-zero on entry to `query_handler`; need to inspect the early exit of the drain loop at drv L241-L245 and how `ForwardThenFlush` for an extended `Execute` maps to `ForwardedMessage` at drv L214-L220.
- Unclear what progresses the backend leg while fe L436 awaits; need to inspect `session.forward_frontend` in pg-proto and the surrounding `select` in `drv` above L195.
- Unclear how `Backend` treats rows of a multi-statement `Query` whose statements have different projections, given one portal per operation; need to inspect `data_row::as_ciphertext` and `flush_encrypted_rows` (be L473-L560) and `get_statement_for_operation` (ctx L1242-L1270).
- Unclear how `Backend` consumes the schema execution queue and settings `Pending` when PostgreSQL's completion count differs from `forwarded_statements.len()`; need to inspect `Backend::intercept_backend` beyond be L180 and `SettingsProtocol::ready` / `failed`.
- Unclear whether AST `Display` round-trips every statement kind sqltk accepts under the PostgreSQL dialect; need to inspect sqltk's `Display` implementations (not in this workspace's source tree as read).
- Unclear which DDL or utility statements outside the `match` at sm L556-L609 can alter what a later `requires_schema` statement resolves (for example `SET search_path`, `ALTER TABLE ... SET SCHEMA`, `CREATE SCHEMA`, `DO`, `CALL`); need to inspect `is_catalog_schema_ddl` (sm L1123), `is_encryption_neutral_alter_operation` (sm L1137) and `is_modelled_ddl` (sm L1169).
- Unclear whether a transformed statement can change `SettingsCommand::from_statement` or `SchemaMiddleware::intent` relative to its parsed form; need to inspect eql-mapper transformation rules behind tcs L164-L182.
- Unclear what `error_to_response` exposes of the carried fallback error returned at fe L472; need to inspect `PostgreSqlErrorHandler::error_to_response`.
- Unclear whether `ConnectionSettings::command_succeeded` with the empty `success_tag` (st L211-L217, sp L57) projects `Commit` and `SET LOCAL` the same way the confirmed path does; need to inspect st L317 onward.
- Unclear whether `inbound_eql::parse` role checks (inbound_eql.rs L31-L90) cover every literal position `literal_is_query_operand` reports; need to inspect `validate_query_metadata` and `validate_storage_metadata`.
- Unclear whether `EncryptionService::encrypt` preserves input order; need to inspect `proxy/zerokms/zerokms.rs`.
