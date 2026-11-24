# Frontend::intercept

File: `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`, L169-L350.
Signature: `pub async fn intercept(&mut self, operation: OperationId, protocol_message: FrontendMessage) -> Result<FrontendMiddlewareOutput, Error>`.

Citation prefixes used below:

- `F` = `postgresql/middleware/frontend.rs`
- `L` = `postgresql/context/lifecycle.rs`
- `C` = `postgresql/context/mod.rs`
- `SP` = `postgresql/context/settings_protocol.rs`
- `PS` = `postgresql/context/prepared_statements.rs`
- `SM` = `proxy/schema/middleware.rs`
- `B` = `postgresql/middleware/backend.rs`
- `M` = `postgresql/middleware/mod.rs`
- `D` = `postgresql/driver.rs`
- `EH` = `postgresql/error_handler.rs`
- `BR` = `postgresql/rewrite/bind.rs`
- `T` = `config/tandem.rs`

## 1. Purpose

`intercept` is the single dispatcher for every frontend protocol message that pg-proto hands to the middleware after startup and authentication.
For each message it returns one of three dispositions: `Forward(message)`, `ForwardThenFlush(message)` or `Suppress(message)` (F177, F183, F186, F210, F346, F348).
It never writes to the client directly.
A local refusal is realised by forwarding a substitute message that PostgreSQL is relied on to reject, and by storing a diagnostic in `Context` that `Backend` swaps into PostgreSQL's `ErrorResponse` (F229-F232, F243-F252, F269-F272, F294-F318; B315-B321).

## 2. Caller and reachability

The only production caller is `CipherStashMiddleware::frontend_operation` (M54-M63), which converts `pg_proto::OperationId` into the Proxy `OperationId` (`postgresql/mod.rs` L30-L34) and awaits `intercept`.
`CipherStashMiddleware` is constructed per connection by `CipherStashMiddlewareFactory::create` (M31-M33) and installed in the pg-proto `Intermediary` at D110.
`Frontend` and `Backend` each hold a clone of the same `Context` (M39-M40).
`Context.protocol_state` is an `Arc<RwLock<ConnectionProtocolState>>` (C67) and every `SchemaMiddleware` field is an `Arc` (SM313-SM329), so both halves observe the same state.
`frontend_operation` and `backend_operation` both take `&mut self` on the one `CipherStashMiddleware` (M54-M76), so the two intercepts never run concurrently on one connection.
The driver comment at D226-D228 states the intermediary is single-task.

An `Err` returned from `intercept` surfaces to the driver as `pg_proto::ForwardError::Middleware(error)` and the driver returns it from the forward loop (D204-D206, D211), which ends `handler` for that connection.

## 3. Inputs

- `operation`: an identifier allocated by pg-proto for this message. `OperationId` derives `Ord` (`postgresql/mod.rs` L20-L21) and callees compare identifiers with `<`, `<=`, `>=` (SP344, SP347, C954, C987, PS122).
- `protocol_message`: a decoded `FrontendMessage`. Content is controlled by the authenticated client. Variants named in this function: `Terminate`, `Sync`, `Query`, `Bind`, `Execute`, `Describe`, `Close`, `Parse`, `FunctionCall` (F176-F207, F216-F343). All other variants fall to `_` arms (F208, F336).
- `self.failed_extended_batch`: Frontend-local `bool`, initialised `false` (F165).
- `self.context`: shared connection state (section 2) plus immutable `Arc<TandemConfig>` (C63).

## 4. Block-by-block

### 4.1 Terminate short-circuit (F176-F178)

`Terminate` is returned as `Forward` before any state is read or written.
No `Context` method is called on this path.
The comment at F174-F175 asserts PostgreSQL processes `Terminate` while ignoring a failed batch; no code in this repository establishes that.

### 4.2 Discard gate (F179-F187)

Condition: `self.failed_extended_batch || self.context.discarding_extended_messages()?`.
`discarding_extended_messages` takes the read lock and returns `settings_protocol.discarding()` (C1550-C1557), which is the `discarding_until_sync` field (SP433-SP435).
A poisoned lock produces `ContextError::ProtocolStateUnavailable` and `intercept` returns `Err` (C1554, F179).

When the gate is closed:

- `Sync` (F180-F184): `failed_extended_batch` is cleared at F181 before `context.sync(operation)?` at F182, so the flag is cleared even when `sync` returns `Err`. `Context::sync` pushes a `ReadyBoundary` into the schema execution queue (L366, C1385-C1387, SM667-SM672) and calls `SettingsProtocol::sync` (L367, C1559-C1566), which sets `discarding_until_sync = false`, pushes `operation` onto `boundaries`, and inserts a barrier `Pending` when `settings_since_boundary || in_flight()` (SP439-SP452). The `Sync` is forwarded unchanged (F183).
- Any other message (F185-F186): `context.discard_operation(operation)?` removes `operations[operation]` if present and releases its metrics scope if unreferenced (C1026-C1043). The message is returned as `Suppress`.

The gate applies to every variant except `Terminate` and `Sync`, including `Query` and `FunctionCall`, because the variant test at F180 only distinguishes `Sync`.
The gate runs before the configuration `mapping_disabled()` test at F189, so suppression applies on mapping-disabled connections as well.

Two independent sources close the gate:

1. `failed_extended_batch`, written `true` only at F245, F271, F296, F306, F317 and written `false` only at F181. No `Backend` code reads or writes it (it is a private field of `Frontend`, F150). `ReadyForQuery` does not clear it.
2. `SettingsProtocol.discarding_until_sync`, set in `SettingsProtocol::failed` when no registered boundary is `>=` the failed operation (SP340-SP345), reached from `Backend` on a PostgreSQL `ErrorResponse` through `postgres_failed` -> `settings_execution_failed` -> `restore_failed_preparation` (B109, L478, C1608-C1613, C220). It is cleared by `sync` (SP440) and by `ready` (SP457).

Boundaries are registered by `sync` (SP441), `simple_query` (SP284), and `readiness_boundary` (L87), so a failure of an operation that registered its own boundary does not set `discarding_until_sync` (SP340-SP345).

### 4.3 Configuration mapping-disabled passthrough (F189-F211)

Condition: `self.context.mapping_disabled()`, which reads `config.development.disable_mapping` (C1794-C1796, T317-T322).
This is the operator configuration flag, not the `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING` connection setting; the connection setting is read by `unsafe_disable_mapping()` (C1630-C1636) inside `route` (F1254) and `begin_bind` (L239), not in `intercept`.

Per-variant bookkeeping, then the original message is forwarded unchanged (F210):

| Variant | Callee | Effect |
|---|---|---|
| `Query` | `unmapped_query` (L58-L63) | Starts a metrics scope, installs an unnamed `Portal::passthrough`, sets an until-readiness `ExecuteContext`, registers a readiness boundary. No SQL is parsed. |
| `Bind` | `unmapped_bind` (L47-L56) | Creates the operation entry and installs `Portal::passthrough` under the client's portal name. |
| `Execute` | `execute` (L306-L310) | `mapping_disabled()` branch: `set_execute_for_portal` only; returns `Ok(false)`. The returned `bool` is dropped at F197. |
| `Describe` | `set_describe` (C490-C512) | Records a `DescribeContext`. |
| `Close` | `close` (L317-L363) | Skips `settings_protocol.close` when mapping is disabled (L323); removes statement or portal metadata. |
| `Parse` | `unmapped_parse` (L42-L45) | Creates the operation entry and removes any stored metadata for that statement name. |
| `FunctionCall` | `function_call` (L75-L77) | Registers a readiness boundary. |
| `Sync` | `unmapped_sync` (L68-L70) | `settings_protocol_boundary` only; no schema `ReadyBoundary` is pushed, unlike `Context::sync` (L365-L368). |
| other | none (F208) | No state change. |

Every callee error propagates with `?` and becomes a connection-level `Err`; this branch produces no local error response.
`failed_extended_batch` is never set on this branch because every assignment to `true` lies after the return at F210.

### 4.4 Mapped dispatch (F213-F343)

`outbound_message` starts as a clone of the input (F213) and `flush_after_forward` starts `false` (F214).

#### Query (F217-F235)

1. `start_metrics_scope()?` allocates a `SessionId` and inserts a `SessionMetricsContext` (C550-C559).
2. `query_handler(operation, query, session_id).await` (F394-F607) returns:
   - `Ok(Some(mapped))`: `outbound_message` becomes the rewritten `Query` (F220).
   - `Ok(None)`: the client's bytes are forwarded unchanged (F222).
   - `Err(err)`: logged (F224-F228), converted with `error_to_response` (F229, EH37-EH75), recorded with `refuse_query` (F230-F231), and `outbound_message` is replaced by `simple_error_query(&response)` (F232).
3. `failed_extended_batch` is not touched on this arm.

`refuse_query` (L384-L397) finishes the metrics scope, stores the diagnostic in `operations[operation].error_response` (C1045-C1058), installs an `ExecuteContext` for the unnamed portal name with no metrics scope via `set_execute` (L392, C640-C655), registers a settings boundary (L394) and a schema `ReadyBoundary` (L395).

`simple_error_query` (F1703-F1729) builds `DO '<body>';` where body is `BEGIN RAISE EXCEPTION USING <options>; END;` and each option value is a diagnostic field passed through `quote_literal` (F1720-F1723); the body is then passed through `quote_literal` a second time (F1728).
`quote_literal` doubles backslashes and single quotes and emits `E'...'` when the input contains a backslash, otherwise `'...'` (F1480-F1487).
Diagnostic field values are built from `err.to_string()` (EH39-EH73), and several error variants interpolate client-supplied text: the prepared-statement name (F1024-F1026), table names (F1362-F1367, F1406-F1410), column names (F1412-F1415), and EQL Mapper error text (F1349, F1390).
On the `Err` path none of the client's original SQL bytes are placed in `outbound_message` (F232 overwrites the clone made at F213).

State that `query_handler` has already written before an `Err` and that `refuse_query` does not undo: `set_deallocations` appends to `operations[operation].deallocations` at F421-F427 before the checks at F429-F438 and the routing at F448-F452.

#### Describe (F236-F238)

`describe_handler` (F352-F360) calls `set_describe` (C490-C512), which resolves the target against `state.portals` or `state.prepared_statements` and stores `DescribeContext { statement }`.
The lookup result is `None` for an unknown name (C500-C507); no refusal exists on this arm.
The message is forwarded unchanged.

#### Execute (F239-F255)

`context.execute(operation, &execute.portal)` (L306-L315), non-disabled path:

1. `execute_settings_portal` (C1495-C1512) -> `SettingsProtocol::execute` (SP232-SP261). Returns `Ok(false)` when the portal name is not in `settings_protocol.portals` (SP239-SP241). Returns `Err(SettingsRefusal)` when the bound statement `needs_mapping` and the settings captured at Bind differ from current settings (SP242-SP249); this is mapped to `MappingError::PreparedSettingsChanged` (C1485-C1493). Otherwise inserts a `Pending` and returns whether the command may change effective settings (SP250-SP260).
2. `execute_schema_portal` (C1366-C1368) -> `SchemaMiddleware::execute` (SM511-SM523): sets `transaction_active`, increments `in_flight_ddl` when the portal's intent changes schema, pushes a `PendingExecution::Execute`, returns whether DDL is executing.
3. `set_execute_for_portal` (C676-C706): resumes a suspended execution or creates a new metrics scope, and attaches `state.portals.get(&name).cloned()` to the `ExecuteContext`; this is `None` when Proxy holds no portal of that name (C682, C696).

`Ok(flush)`: `flush_after_forward = flush` (F241); the `Execute` is forwarded unchanged.

`Err(err)` (F242-F253): `refuse_execute` stores the diagnostic and installs an `ExecuteContext` keyed to the unnamed portal name, not the portal the client named (L399-L406).
`failed_extended_batch = true` (F245).
`outbound_message` is an `Execute` whose `portal` is `refusal_name()` and whose `max_rows` is the client's (F246-F251).
`flush_after_forward = true` (F252).
`refusal_name` is `"__cipherstash_" + Uuid::new_v4()` (F1756-F1760).

The error return at step 1 happens before steps 2 and 3, so a refused Execute adds no schema `PendingExecution` and no portal-derived `ExecuteContext` (L311-L313).

No check on this arm tests whether Proxy holds a portal with the requested name; an unknown name proceeds through all three steps and is forwarded (SP239-SP241, SM512-SM521, C682-C697).
By contrast, `bind_handler` refuses a statement name with no recorded Parse (F1023-F1028).

#### Parse (F256-F275)

`failed_parse` is a clone of the client's `Parse` (F257).
`parse_handler(operation, parse).await` (F804-F902):

- `Ok(Some(mapped))`: rewritten `Parse` forwarded (F259-F261).
- `Ok(None)`: original forwarded (F262).
- `Err(err)`: logged, `error_to_response`, `refuse_parse(operation, response.clone())?` (F269-F270), `failed_extended_batch = true` (F271), `outbound_message = extended_parse_error(failed_parse, &response)` (F272).

`refuse_parse` (L208-L229) takes `operations[operation].preparation`; it returns `ContextError::UnknownOperation` when no `Preparation::Parse` is present (L218-L225), which propagates through `?` at F270 as a connection-level `Err`.
`begin_parse` installs that preparation (L113-L117) and is the first fallible step of `parse_handler` (F810-F812); `finish_parse` takes it (L186-L195, called at F876 and F880).
`refuse_parse` then removes the statement name's metadata (`close_statement`, L227, C784-C796) and stores the diagnostic (L228).

`extended_parse_error` (F1731-F1743) keeps the client's statement name, replaces the SQL with `SELECT NULL::"__cipherstash_<uuid>"`, and clears `parameter_types`.
The client's SQL is not forwarded on this path.
`flush_after_forward` stays `false` on this arm.

#### Bind (F276-F322)

`failed_bind` is a clone of the client's `pg_proto::Bind` (F277); the portal and statement names are cloned (F278).
`Bind::try_from(bind)?` (F279, BR348-BR395) returns `ProtocolError::ParameterFormatCodesMismatch` when the number of parameter format codes is not 0, 1, or the number of parameters (BR363-BR373).
That error leaves `intercept` through `?` at F279: it is not converted to a local error response, `refuse_bind` is not called, and `failed_extended_batch` is not set.

`bind_handler(operation, bind).await` (F1003-F1080):

- `Ok(Some(mapped))`: `bind_statement_effects(&portal, &statement)?` then rewritten `Bind` forwarded (F280-F283).
- `Ok(None)`: `bind_statement_effects` then original forwarded (F285-F287). This includes the connection-setting passthrough at F1011-F1017, which returns before the `is_prepared` check at F1023.
- `Err(err)`: three match arms (F289-F319) that differ only in the log message; each calls `error_to_response`, `refuse_bind(operation, response.clone())?`, sets `failed_extended_batch = true`, and sets `outbound_message = extended_bind_error(failed_bind, &response)`.

`bind_statement_effects` (C814-C824) copies the prepared statement's `Deallocation` onto `portal_operations[portal]` when that entry exists.

`refuse_bind` (L283-L304) takes the preparation if present, finishes its metrics scope, and stores the diagnostic.
It tolerates an absent preparation (L293-L300), which is the state when `begin_bind` fails inside `bind_settings_statement` before L250.

`extended_bind_error` (F1745-F1754) replaces only `bind.statement` with `refusal_name()`.
The portal name, parameter format codes, parameter values, and result formats in the forwarded message are the client's originals from the clone at F277.
`flush_after_forward` stays `false` on this arm.

State on the `begin_bind` refusal path: `SettingsProtocol::bind` removes `portals[portal]` and journals the previous value (SP200-SP209) before returning `Err(refusal)` (SP213-SP219), while `state.portals[portal]` in `ConnectionProtocolState` is untouched until `Backend` runs `restore_failed_preparation` (C241-C261).

#### Sync (F323-F329)

`context.sync(operation)?` as described in 4.2.
Forwarded unchanged.

#### Close (F330-F332)

`context.close(operation, &close)?` (L317-L363) journals the close with a rollback snapshot (L324-L333, SP524-SP544) and removes the statement or portal metadata immediately (L335-L357).
Forwarded unchanged.

#### FunctionCall (F333-F335)

`context.function_call(operation)?` registers a readiness boundary (L75-L89).
The message body is not inspected and is forwarded unchanged.

#### Other variants (F336-F342)

Logged at debug and forwarded unchanged with no state change.
No COPY sub-protocol message is named in this function; the COPY policy lives in `route` at Query/Parse time (F1257-F1262, F1400-F1455).

### 4.5 Return (F345-F349)

`ForwardThenFlush(outbound_message)` when `flush_after_forward`, else `Forward(outbound_message)`.
`flush_after_forward` is `true` only from the Execute arm (F241, F252).

## 5. Error-propagation classes

Two disjoint classes leave this function.

1. **Local refusal** (returns `Ok`): handler errors from `query_handler`, `parse_handler`, `bind_handler`, and `context.execute` in the mapped path (F223, F242, F263, F288).
2. **Connection-level `Err`**: every `?` in `intercept`: F179, F182, F185, F191-F207, F218, F231, F237, F244, F270, F279, F281, F286, F295, F305, F316, F328, F331, F334.

`Bind::try_from` (F279) and `describe_handler` (F237) are the two handler-shaped calls that belong to class 2.

## 6. Invariants

1. `Terminate` is forwarded regardless of all state (F176-F178).
2. While `failed_extended_batch` is `true` or `discarding_until_sync` is `true`, only `Terminate` and `Sync` are forwarded; every other message is returned as `Suppress` (F176-F187).
3. `failed_extended_batch` becomes `true` on every local refusal of `Execute`, `Parse` and `Bind` (F245, F271, F296, F306, F317) and on no other path.
4. `failed_extended_batch` becomes `false` only on a `Sync` that reaches F181.
5. On each local refusal the diagnostic is stored in `Context` before the substitute message is assigned, and a failure to store it aborts with `Err` (F230-F232, F244-F246, F270-F272, F295-F297, F305-F307, F316-F318).
6. On a refused `Query` or `Parse`, the forwarded message contains none of the client's SQL (F232, F1728; F272, F1740-F1741).
7. With configuration mapping disabled, the forwarded message is the unmodified input for every variant (F210).
8. `ForwardThenFlush` is returned only for `Execute` (F241, F252, F345).
9. Refusal object names contain a fresh UUIDv4 per call (F1756-F1760).

## 7. Assumptions

1. PostgreSQL rejects each substitute message with an `ErrorResponse`: the `DO ... RAISE EXCEPTION` block (F1727-F1728), the cast to a type named `__cipherstash_<uuid>` (F1740), the Bind of a statement named `__cipherstash_<uuid>` (F1752), and the Execute of a portal named `__cipherstash_<uuid>` (F249). Nothing in this repository establishes it. If the substitute completes instead, `finish_execution` with `Completed` returns `replacement_error: None` and removes the operation (C363-C372), so the stored diagnostic is dropped.
2. pg-proto correlates that `ErrorResponse` to the same `operation`; `Backend` substitutes the stored diagnostic only when `operation.is_some()` (B317-B321) and only through `finish_execution(…, Failed)` taking `error_response` (C363-C365). pg-proto source is not available locally.
3. `Suppress` prevents the message from reaching PostgreSQL. pg-proto source is not available locally.
4. `quote_literal` (F1480-F1487) yields a single SQL string literal for every diagnostic value under the upstream server's string-literal settings. The function handles `\` and `'` only; nothing found for other bytes (for example NUL after `from_utf8_lossy` at F1722).
5. The upstream role can execute an anonymous `DO` block in the default procedural language (F1727-F1728). Nothing found.
6. Every portal PostgreSQL will execute by name was created by a Bind that Proxy processed. The Execute arm has no existence check (F240; SP239-SP241; C682-C697). Nothing found.
7. Messages that reach the `_` arm (F336) and `FunctionCall` (F333) carry no values that need encryption. Nothing found in this function; COPY-in is addressed only by `route` (F1257-F1262).
8. `operation` identifiers increase in arrival order, so the ordered comparisons in the journal and boundary logic are meaningful (SP344-SP347, C954, C987, PS122). Established only by pg-proto.
9. PostgreSQL's own discard-until-Sync state matches the union of `failed_extended_batch` and `discarding_until_sync` (comment at L65-L67 and SP456). Nothing in this repository establishes it.
10. `Context::mapping_disabled()` is constant for the life of a connection; `config` is an `Arc<TandemConfig>` fixed at `Context` construction (C63, C478-C482).
11. PostgreSQL emits the `ErrorResponse` for a substituted `Parse` or `Bind` without a flush being requested (those arms leave `flush_after_forward` false, F256-F322), while the refused `Execute` requests one (F252). Nothing found that explains the difference.

## 8. Shared state touched

Read: `settings_protocol.discarding_until_sync` (C1550-C1557), `config.development.disable_mapping` (T317-T322).
Written directly: `Frontend.failed_extended_batch` (F181, F245, F271, F296, F306, F317).
Written through callees: `ConnectionProtocolState.operations`, `.portals`, `.portal_operations`, `.prepared_statements`, `.statement_metrics`, `.suspended_executions`, `.settings_protocol` (`journal`, `pending`, `boundaries`, `portals`, `prepared`, `discarding_until_sync`), `SchemaMiddleware.executions`, `.in_flight_ddl`, `.transaction_active`, `.prepared`, `.portals`, and `Context.session_id_counter` (C551, C677).

## 9. Cross-function dependencies

- `Backend::intercept_backend` consumes `operations[op].error_response` (B254, B311-B321) and drives `SettingsProtocol::failed`, which sets the shared discard flag read at F179.
- The driver drains backend outcomes after a forwarded `Query`, `Sync`, or expanded frontend message while schema DDL or settings are in flight (D214-D267). `query_handler` and `prepare_parse` await `wait_for_schema_execution` (F436, L150; SM675-SM683), which completes only when `in_flight_ddl` reaches zero; that counter is decremented from the Backend half.
- `route` (F1248-F1295) holds the per-statement map/forward/refuse decision; `intercept` only converts its `Err` into a refusal.

## 10. Open questions

See the structured record.
