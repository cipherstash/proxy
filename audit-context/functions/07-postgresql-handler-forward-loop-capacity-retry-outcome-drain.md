# postgresql::handler (forward loop, capacity retry, outcome drain)

File: `packages/cipherstash-proxy/src/postgresql/driver.rs`, L85-L342, plus the helpers it calls in the same file (L344-L375) and the adapter types at L23-L83.
Signature: `pub async fn handler<S>(client_stream: TcpStream, context: Context<S>) -> Result<(), Error> where S: EncryptionService + Clone`.
All bare line references are to `driver.rs`.
Other files are named explicitly and are relative to `packages/cipherstash-proxy/src/`.

Note on format: `ANALYSIS_FORMAT.md`, `FUNCTION_MICRO_ANALYSIS_EXAMPLE.md` and `DOMAIN_NOTES.md` were not found on disk (glob for `**/audit-context-building/resources/*.md` under the repo and under `~/.claude` returned nothing), so this document follows the section order of the existing `04-ConnectionSettings-command_succeeded.md`.

Note on pg-proto: `Cargo.toml:32` pins `pg-proto = "0.12.0"`.
No `pg-proto-*` directory exists under `~/.cargo/registry/src/index.crates.io-1949cf8c6b5b557f/`, so every `pg_proto::` item is a black box here.
Every statement about pg-proto below is either what `driver.rs` sends to it, or an assumption with "nothing found" as its source.

## 1. Purpose

`handler` is the whole lifetime of one client connection.
It does four things in order:

1. Selects the downstream (client-facing) TLS policy and the upstream (database-facing) TLS policy and credentials from operator configuration (L89-L98, L282-L341).
2. Builds a pg-proto `Intermediary` with the CipherStash middleware and runs `accept`, which covers startup, SSLRequest, authentication, the upstream connection and CancelRequest forwarding (L103-L124).
3. Drives the session with a single-task loop that calls `session.forward_next()` once per iteration (L130-L274).
4. Adds two secondary loops on top of `forward_next`: the capacity retry (L140-L210) and the outcome drain (L214-L267).

The code is written as two nested `macro_rules!` (`run!` L100-L280, `run_client!` L282-L313) because the `Server` and `Client` builder types differ per TLS policy.
The macro body is expanded six times (three server policies at L324, L330, L337, times two client policies at L294, L310); all six expansions are textually identical apart from `$server` and `$client`.

## 2. Inputs and who controls them

| Input | Source | Controlled by |
| --- | --- | --- |
| `client_stream` | `connect::accept` (`connect/mod.rs:48-52`), passed at `main.rs:116` | Any network peer that reaches the listener |
| `context` | `proxy.context(client_id)` at `main.rs:110` (`proxy/mod.rs:134`) | Operator config captured at `Proxy::init`; per-connection protocol and schema state starts empty |
| `context.config` values read here | `require_tls` (`context/mod.rs:1838`), `tls_config` (`:1830`), `database_socket_address` (`:1818`), `database_username` (`:1822`), `database_password` (`:1826`), `connection_timeout` (`:1790`), `database_tls_disabled` (`:1846`), `config().database.server_name()` (`config/database.rs:88`), `with_tls_verification` (L356), `upstream_tls_roots` (`context/mod.rs:1854`) | Operator |
| `context.schema_ddl_in_flight()`, `context.settings_in_flight()` | Per-connection state written by the middleware this handler installs at L110 | Authenticated client SQL (increments), upstream PostgreSQL responses (decrements) |
| Results of `forward_next`, `forward_backend`, `forward_frontend` | pg-proto | Client bytes, upstream bytes, pg-proto's own classification |

`context` is `Clone` and every mutable field is behind an `Arc` (`context/mod.rs:57-70`: `protocol_state: Arc<RwLock<..>>`, `schema_middleware` whose fields are `Arc`s at `proxy/schema/middleware.rs:319-325`).
The handler's own `context` (read at L220, L258-L259) and the clones handed to `Frontend` and `Backend` (`middleware/mod.rs:32`, `:39-40`) therefore observe the same state.
This sharing is what makes the handler's in-flight polls meaningful.

## 3. Callers

`main.rs:116`, inside `tracker.spawn` (`main.rs:112`), one task per accepted socket.
No other caller exists (`grep "handler\("` over the package returns only `main.rs:116` and the definition).
The caller maps the returned error to a log line only (`main.rs:119-137`): `ConnectionTimeout` logs at warn, everything else at error.
Nothing in the caller writes to the client socket after `handler` returns.

## 4. Block-by-block

### 4.1 Downstream TLS precondition, L89 and L344-L349

`validate_downstream_tls(require_tls, tls_configured)` returns `ConfigError::TlsConfigurationRequired` when `require_tls && !tls_configured`.
The same predicate is checked at config build time (`config/tandem.rs:284-286`, called at `:139` and `:277`).
L89 is therefore a second, per-connection check of a condition the config loader already rejects.
After L89 the combination `require_tls == true && tls_config == None` cannot reach L337.

### 4.2 Credentials, L90-L98

`address` is `"{host}:{port}"` from `config.database` (`config/database.rs:57-59`).
`downstream_auth = StaticMd5ServerCredentials::new(username, password)` (L91-L94) and `upstream_auth = StaticClientCredentials::new(username, password)` (L95-L98) are both built from the same two values: `config.database.username` and `config.database.password()`.
`password()` unwraps a `Protected<String>` into a plain `String` (`config/database.rs:74-76`); two plain copies exist per connection, one in each credentials object, and `upstream_auth` is cloned again at L291 or L307.

Structural facts that follow:

- The secret a client must present to Proxy is the database password itself; there is no separate Proxy credential.
- The downstream mechanism is fixed to MD5 by the type name at L91; no other mechanism is offered by this file.
- Every upstream session authenticates as `config.database.username` regardless of what the client sent in its startup packet.
- `Route::resolve` (L50-L59) ignores both `StartupParameters` and `InitialServerContext` and always returns the configured address.

What pg-proto does with the client's startup `user`, `database` and `options` parameters is not visible in this file (see open questions).

### 4.3 Downstream TLS policy selection, L320-L341

| `tls_config()` | `require_tls()` | Policy | Line |
| --- | --- | --- | --- |
| `Some` | `true` | `ServerTlsPolicy::Required(identity)` | L323-L328 |
| `Some` | `false` | `ServerTlsPolicy::Optional(identity)` | L330-L334 |
| `None` | (false, by L89) | `ServerTlsPolicy::Disabled` | L337-L341 |

`tls::configure_server_with_leaf(tls_config)?` at L321 runs on every connection.
For `TlsConfig::Path` it reads the certificate and key files from disk each time (`tls/mod.rs:56`, `:67`); for `TlsConfig::Pem` it parses the in-memory strings (`tls/mod.rs:51`, `:63`).
Its error paths are: PEM parse failure (`tls/mod.rs:51`, `:56`, `:68`), empty certificate list (`tls/mod.rs:70-73`), and `with_single_cert` rejection (`tls/mod.rs:76`).
On any of them `handler` returns `Err` at L321 before any byte is exchanged with the client.
`with_no_client_auth()` (`tls/mod.rs:75`) means no client certificate is requested.

With `Optional` and `Disabled`, the MD5 exchange from 4.2 can run over a plaintext socket.
`main.rs:206-208` logs a warning at startup for the `NotConfigured` case.

### 4.4 Upstream client and TLS policy, L282-L313 and L355-L363

`database_tls_disabled()` is `config.development.disable_database_tls` (`config/tandem.rs:310-315`).

- When true: `ClientTlsPolicy::Disabled` (L290). The upstream link is plaintext, including the upstream authentication exchange.
- When false: `ClientTlsPolicy::libpq(mode, provider)` (L306) with `mode = VerifyFull` if `config.database.with_tls_verification` else `SslMode::Prefer` (L356-L362).

`with_tls_verification` defaults to `false` (`#[serde(default)]` at `config/database.rs:26-27`), so the default upstream mode is `Prefer`.
The comment at L359-L360 states the intended semantics of `Prefer`: attempt SSL, continue in plaintext when the database rejects it.
Whether pg-proto validates the certificate chain or the server name under `Prefer` is not established by any line in this repository.

`UpstreamTls` (L70-L83) supplies `server_name` and a clone of the whole `RootCertStore` per resolve (L80).
`server_name` comes from `config.database.server_name()?` at L297, which fails with `InvalidServerName` when `host` is not a valid DNS name or IP (`config/database.rs:88-95`).
That `?` returns from `handler` per connection; `main.rs:170` validates `config.server.server_name()` at startup, which is the listener host, not the database host.
The roots are the native store loaded once at `Proxy::init` (`proxy/mod.rs:62-68`); an empty store is rejected at init only when `with_tls_verification` is true (`proxy/mod.rs:65-67`).

The connector closure (L286-L289, L302-L305) calls `connect::connect(target.name())`.
`connect::connect` (`connect/mod.rs:54-75`) retries `TcpStream::connect` up to `MAX_RETRY_COUNT = 3` more times with sleeps of 100, 200, 400 ms, and returns `Error::DatabaseConnection` after the fourth failure (`connect/mod.rs:68-71`).
It has no timeout of its own; it is bounded only by the outer accept timeout at L115 when one is configured.

A separate upstream path exists for the schema loader (`connect::database`, `connect/mod.rs:77-104`), which installs `NoCertificateVerification` when `with_tls_verification` is false (`tls/mod.rs:89-92`).
That path does not go through `handler`; it is noted because the two upstream paths derive their TLS behaviour from the same flag through different code.

### 4.5 Intermediary construction and accept, L102-L124

Builder inputs: `Route(address)` (L106), `CancellationPolicy::Forward` (L107), a clone of the process-global `CANCELLATION_REGISTRY` (L108, defined L25-L26), `BoundedPipeline::new(256)` (L109), and `CipherStashMiddlewareFactory(context.clone())` (L110).
`.expect("non-zero pipeline bound")` at L109 is on a literal `256`.
`build()` errors are mapped through `invalid_data` (L112, L351-L353), which stringifies the error into an `io::Error` of kind `InvalidData`; the original error type is lost.

`intermediary.accept(client_stream, (), ())` (L113) is awaited under `tokio::time::timeout(duration, ..)` when `connection_timeout` is `Some` (L114-L119).
`connection_timeout()` is `None` only when the operator sets `0`; unset means 120 000 ms (`config/database.rs:78-86`).
The single timeout covers everything inside `accept`: client startup, optional TLS handshake, MD5 exchange, upstream TCP connect with retries, upstream TLS, upstream authentication.
On timeout the `accept` future is dropped and `handler` returns `Error::ConnectionTimeout` (L117).

`accept` has three outcomes at L120-L124:

- `Err(_)` goes through `invalid_data` and returns.
- `IntermediaryAccept::CancellationForwarded` returns `Ok(())` at L123. No session exists, no guard is created, the middleware factory's handler is never used.
- `IntermediaryAccept::Session(session)` continues.

All authentication failures surface to the caller as `InvalidData` io errors (L120); the handler does not distinguish them.

### 4.6 Cancellation guard, L125-L128 and L28-L45

`CancellationRegistrationGuard` holds a clone of the registry and `session.cancellation_key().cloned()`.
Its `Drop` calls `registry.detach(&key)` and discards the result (L41-L43).
The guard is a local of the macro's block expression, so it is dropped on every exit from the block after L125: normal completion, each `return Err(..)` inside the inner `async` block (those return from the async block, not from `handler`), and task cancellation.

The guard is created only after `accept` has returned `Session`.
Three exits happen before it exists: the timeout at L117, the error at L120, and `CancellationForwarded` at L123.
Which component registers the key, and at what point inside `accept`, is not visible here; the system context records "pg-proto on session accept".
If registration happens inside `accept` before a later step of `accept` fails or the timeout drops the future, this file contains nothing that detaches the key.

`CancellationPolicy::Forward` with a process-global registry means a CancelRequest arriving on any new connection is resolved against keys of all live sessions.
The lookup and the comparison of the presented secret are inside pg-proto.

### 4.7 Primary loop, L129-L139 and L268-L277

Each iteration creates `session.forward_next()` (L131) and awaits it, under the same `connection_timeout` when configured (L132-L137).
The timeout is per await, so on the primary loop it is an idle timeout over both directions: it fires when pg-proto completes no forward for `duration`.
A timeout at L135 returns `Err` from the async block through `?`; the loop does not continue and `session` is not used again.
The same holds for every other timeout site (L154, L175, L238).
Consequence: no `forward_*` future that was dropped by a timeout is ever followed by another call on the same session, so the handler does not rely on those futures being cancel-safe.

Result handling at L138-L213:

- `Ok(forwarded)` passes through.
- `Err(ForwardError::Frontend(FrontendProjectionError::Capacity(_)))` enters the capacity retry (4.8).
- `Err(ForwardError::Middleware(error))` returns the middleware's own `Error` unchanged (L211).
- Any other `Err` is stringified by `invalid_data` (L212).

Loop exit: `ForwardedMessage::Frontend(FrontendMessage::Terminate)` breaks at L268-L273, and the capacity path breaks `'session` directly at L182.
`Frontend::intercept` returns `Forward(Terminate)` before any other check (`middleware/frontend.rs:176-178`), so a Terminate is never suppressed by the middleware.
There is no other normal exit; client EOF and upstream EOF arrive as `Err` from `forward_next` and leave through L212.

The loop treats every other `ForwardedMessage` variant (backend-direction variants, `FrontendSuppressed`, `FrontendLocallyHandled`, plain `Frontend(_)` for other messages) identically: no action, next iteration.
The `matches!` at L214-L219 and L268-L271 are non-exhaustive by construction, so the full variant set of `ForwardedMessage` is not enumerated anywhere in this file.

### 4.8 Capacity retry, L140-L210

Entered when `forward_next` reports that the frontend message could not be projected because the 256-entry pipeline (L109) is full.
The comment at L143-L148 states the contract assumed of pg-proto: the rejected frontend message is retained and `forward_frontend()` retries that exact message.

Each iteration of the inner `loop` (L149-L209) does:

1. `session.forward_backend()` under the timeout (L150-L156).
   `Ok` with any of the four `BackendForwarding` variants continues (L158-L163).
   `Err(Middleware)` and other errors return (L164-L167).
2. `session.forward_frontend()` under the timeout (L170-L177).
   - `Forwarded(Terminate)` breaks the outer `'session` loop (L179-L183), skipping the drain.
   - `Forwarded(message)`, `Expanded{..}`, `Suppressed(message)`, `LocallyHandled(message)` break the inner loop with the equivalent `ForwardedMessage` variant (L184-L200), which then flows into the drain check at L214 exactly as a `forward_next` result does.
   - `Err(Capacity)` loops again (L201-L203).
   - Other errors return (L204-L207).

Properties of this block:

- Step 1 always runs before step 2, so one backend message is forwarded per retry attempt and at least one before the first retry.
- Step 1 does not call `backend_needs_frontend`; the capacity loop has no exit for a backend that is waiting on client input.
- Step 1 waits for a backend message unconditionally. With `connection_timeout == None` that await has no bound in this file.
- The match at L158-L163 lists all four `BackendForwarding` variants with no wildcard `Ok(_)`, so the set is closed at compile time against the 0.12.0 enum.
- Backend messages forwarded in step 1 run through `Backend::intercept` (`middleware/mod.rs:73-75`) and mutate the shared state. The retried frontend message's `frontend_operation` call is the open point: whether pg-proto invoked the middleware before raising `Capacity` on the first attempt, and whether it invokes it again on retry, is not visible. See open questions.

### 4.9 Outcome drain, L214-L267

#### Trigger

The drain runs when both hold:

1. `forwarded` is `FrontendExpanded{..}`, `Frontend(Query(_))` or `Frontend(Sync)` (L214-L219).
2. `context.schema_ddl_in_flight() || context.settings_in_flight()?` (L220).

The `||` short-circuits, so `settings_in_flight` (which takes the `protocol_state` read lock, `context/mod.rs:1621-1628`) is not called when DDL is in flight.
A poisoned lock returns `ContextError::ProtocolStateUnavailable` and ends the session (L220 `?`).

The drain runs after the triggering frontend message has already been processed by `Frontend::intercept` and forwarded.
It protects the next frontend message, not the one that triggered it.

How each trigger shape arises in the middleware:

- `FrontendExpanded`: the only place the Frontend asks for more than a single forward is `ForwardThenFlush` at `middleware/frontend.rs:345-346`, set by the Execute branch only (`:241`, `:252`). `flush_after_forward` is true when `Context::execute` returns true, i.e. `settings || schema` (`context/lifecycle.rs:311-314`), or when the Execute was refused locally (`middleware/frontend.rs:252`). That pg-proto reports a `ForwardThenFlush` output as `ForwardedMessage::FrontendExpanded` is assumed by L216 and by the comment at L222; nothing in this repository establishes it.
- `Frontend(Query(_))`: any simple query, including the locally built error query from `simple_error_query` (`middleware/frontend.rs:232`).
- `Frontend(Sync)`: any Sync, including one forwarded while the Frontend is in its failed-batch mode (`middleware/frontend.rs:180-183`).

Shapes that do not trigger the drain even when the flags are set: `Frontend(Execute)` returned as plain `Forward` (`middleware/frontend.rs:348`), `Frontend(Parse|Bind|Describe|Close|Flush|FunctionCall|CopyData|CopyDone|CopyFail)`, `FrontendSuppressed`, `FrontendLocallyHandled`, and every backend-direction result of `forward_next`.

#### What the two flags mean

`schema_ddl_in_flight` is `in_flight_ddl != 0` (`proxy/schema/middleware.rs:686-688`).
Writers of the counter:

- Incremented on the frontend path by `SchemaMiddleware::execute` when the portal's intent changes schema (`proxy/schema/middleware.rs:515-517`) and by `simple_query` once per schema-changing statement (`:529-533`).
- Decremented on the backend path by `execution_succeeded` (`:811`), `execution_failed` (`:943`) and `discard_skipped_executions` at ReadyForQuery (`:977-980`).

`settings_in_flight` is `pending.values().any(|p| p.barrier)` (`context/settings_protocol.rs:521-523`).
Writers of barrier entries:

- `SettingsProtocol::execute` inserts `barrier = command.may_change_effective(connection)`, `until_ready: false` (`:250-259`).
- `simple_query` inserts one entry per Query with `until_ready: true` (`:276-283`).
- `sync` inserts `barrier: true, until_ready: true` when `settings_since_boundary || in_flight()` (`:442-450`).
- Removed by `succeeded` when not `until_ready` and commands are empty (`:291-293`), by `failed` for skipped non-`until_ready` or non-barrier entries (`:354-364`), and by `ready` for every id `<= operation` (`:459`).

Three removal paths are conditional on the backend message carrying an operation id:

- `settings_execution_succeeded` returns at once when `operation` is `None` (`context/mod.rs:1573-1575`).
- `SettingsProtocol::failed` returns at once when `operation` is `None` (`context/settings_protocol.rs:337-339`).
- `SettingsProtocol::ready` skips the `retain` when `operation` is `None` (`:458-459`).

An `until_ready` barrier entry is removed only by `ready` with an operation id `>=` its own.
So `settings_in_flight` turning false after a simple Query or a Sync depends on pg-proto correlating the matching `ReadyForQuery` with an operation id that is not lower than the one the entry was inserted under.
That correlation is inside pg-proto.

With `config.mapping_disabled()`, `Context::execute` returns `Ok(false)` without touching either tracker (`context/lifecycle.rs:307-310`), `unmapped_query` registers no settings or schema statements (`:58-63`), and the Frontend always returns plain `Forward` (`middleware/frontend.rs:189-211`).
`unmapped_sync` still calls `settings_protocol.sync` (`context/lifecycle.rs:68-70`), which inserts a barrier only when one is already present or `settings_since_boundary` is set (`context/settings_protocol.rs:442`).

#### Loop body, L232-L266

The loop is boxed (`Box::pin`, L232) and constructed only when the trigger holds; the comment at L229-L231 gives stack-frame size as the reason.
Each iteration:

1. Awaits `session.forward_backend()` under the timeout (L234-L240).
2. If the result is `Ok` and `backend_needs_frontend` is true, breaks (L241-L245).
3. Otherwise `Ok(any variant)` continues, `Err(Middleware)` returns the error, other `Err` returns `invalid_data` (L246-L257).
4. Breaks when neither flag is set (L258-L262).

`backend_needs_frontend` (L365-L375) inspects the pg-proto-reported message: the forwarded or suppressed message, or the `source` of an expansion; `Held` is false.
It is true for `CopyInResponse` and `CopyBothResponse` only.
For `Forwarded`, the inspected message is what pg-proto reports as forwarded; whether that is the upstream original or the middleware's replacement (`outbound_message`, `middleware/backend.rs:372`) is not visible here.
The Backend does not rewrite Copy responses (they fall to the `_` arm at `middleware/backend.rs:362-368`), so both readings give the same answer for these two message types unless a row flush prefix turns the output into `Expand` (`:373-375`), in which case `source` is used (L368).

#### What the drain guarantees, and where the guarantee stops

Guarantee stated by the comment at L222-L228: settings and schema outcomes of a flushed operation are applied before any dependent frontend message is accepted, and the single task keeps reading the backend so a frontend handler never waits on an outcome that only this task can read.

The second half refers to `SchemaMiddleware::wait_for_ddl` (`proxy/schema/middleware.rs:675-683`), which awaits `execution_finished` until `in_flight_ddl == 0`.
It is called from the frontend path at `middleware/frontend.rs:436` (simple query that requires schema) and `context/lifecycle.rs:150` (Parse that requires schema).
`execution_finished.notify_waiters()` is called only from backend-path functions (`proxy/schema/middleware.rs:812`, `:944`, `:980`).
`frontend_operation` and `backend_operation` both take `&mut self` on the same `CipherStashMiddleware` (`middleware/mod.rs:54`, `:65`), so they cannot run concurrently.
A frontend handler that enters `wait_for_ddl` with the counter non-zero therefore stays pending for as long as pg-proto keeps that `frontend_operation` future un-completed, and the only bound in this file is the `connection_timeout` at L133, L173.

The guarantee holds on these paths:

- Execute of a schema-changing or settings-barrier portal: the Frontend returns `ForwardThenFlush`; if that surfaces as `FrontendExpanded`, the drain runs until the Execute's `CommandComplete` or `ErrorResponse` has passed through the Backend.
- Sync after a barrier: the `sync` entry holds `settings_in_flight` true until `ReadyForQuery`.
- Simple Query with a settings barrier: the `until_ready` entry holds until `ReadyForQuery`.

The guarantee stops, or is weaker, on these paths; each is a structural fact with its line:

1. **Copy early exit, L241-L245.**
   The drain breaks with one or both flags still set.
   Control returns to the primary loop, which reads the client's `CopyData`, `CopyDone` and anything pipelined after them through `Frontend::intercept`.
   None of `CopyData`, `CopyDone`, `CopyFail` matches L214-L219, so the drain is not re-entered for them.
   It is re-entered only after the next `Query`, `Sync` or expanded message has itself been intercepted and forwarded.
   That next message is therefore handled by the Frontend while the earlier operation's outcome is unread, unless `forward_next` happens to deliver the backend's `CommandComplete`/`ReadyForQuery` first.
   The order in which `forward_next` services the two directions is inside pg-proto.
   If that next message requires schema and `in_flight_ddl != 0`, it reaches `wait_for_ddl` as described above.

2. **DDL-only simple Query exits before ReadyForQuery.**
   For a Query whose statements change schema but carry no settings barrier, `settings_in_flight` is false and `in_flight_ddl` reaches zero at the last DDL statement's `CommandComplete` (`proxy/schema/middleware.rs:811`).
   L258-L262 breaks there.
   The `ReadyForQuery` that runs `became_ready` (`context/lifecycle.rs:500-519`: settings `ready_for_query`, `publish_schema_if_changed` at Idle, schema `ready_for_query` with `adopt_latest`) has not been read when the primary loop resumes.
   The next frontend message can be intercepted before that `ReadyForQuery` is processed.
   The Frontend's own `prepare_schema_for_statement` (`context/mod.rs:1421-1433`) is what handles publication on that path.
   The same shape applies to an extended Execute of DDL: the drain ends at `CommandComplete`, before any `ReadyForQuery`.

3. **Unflushed Execute.**
   `Forward(Execute)` (flag false at `middleware/frontend.rs:241`) does not trigger the drain.
   A portal for which `SettingsProtocol::execute` returns `Ok(false)` because the portal is unknown to it (`context/settings_protocol.rs:239-241`) and whose schema intent is absent or non-schema-changing (`proxy/schema/middleware.rs:512-513`) takes this path.
   Whether a statement is a barrier is decided at Execute time from the then-current settings (`may_change_effective(connection)`, `context/settings_protocol.rs:251`).

4. **The trigger is evaluated once per forwarded message, after it.**
   Frontend messages that pg-proto reports through `forward_next` as backend-direction results, or that arrive while the flags are set but are not one of the three shapes, pass without a drain.
   Example shape: `Parse`, `Bind`, `Describe`, `Close`, `Flush` following a Copy early exit.

5. **The drain reads only the backend.**
   While it runs, the client socket is not read by this task.
   If the backend produces no further message that clears the flags and is not waiting on Copy input, the loop ends only through the timeout at L236-L238, or not at all when `connection_timeout` is `None`.
   The three operation-id-conditional removal paths listed above are the places where a flag can stay set after the backend has in fact answered.

6. **Terminate inside the capacity retry skips the drain** (L179-L183), which is consistent with L268-L273 since the session ends either way.

### 4.10 Error mapping, L351-L353

`invalid_data` converts any `Display` error into `io::Error(InvalidData)`.
It is applied to builder errors (L112, L293, L309, L328, L334, L341), accept errors (L120), and all non-middleware forward errors (L167, L207, L212, L256).
Client disconnect, upstream disconnect, authentication failure, protocol violation and TLS failure are therefore indistinguishable to `main.rs:119-137`, which matches on `Error::ConnectionClosed`, `Error::CancelRequest` and `Error::ConnectionTimeout` only.
Whether `Error::ConnectionClosed` can still be produced on any path from this function is not established by this file (the three arms at L211, L164, L204, L253 are the only ones that pass a typed `Error` through).

## 5. Invariants

1. `require_tls && tls_config.is_none()` never reaches session setup. Evidence: L89, L344-L348.
2. Downstream policy is `Required` iff `tls_config.is_some() && require_tls`; `Optional` iff `tls_config.is_some() && !require_tls`; `Disabled` iff `tls_config.is_none()`. Evidence: L320-L341.
3. Upstream policy is `Disabled` iff `development.disable_database_tls`; otherwise `VerifyFull` iff `database.with_tls_verification`, else `Prefer`. Evidence: L284-L310, L355-L363.
4. Downstream and upstream credentials are the same username and password. Evidence: L91-L98.
5. The upstream target is always the configured `host:port`; client startup parameters do not influence it. Evidence: L50-L58, L106.
6. Exactly one task drives both directions of a session; frontend and backend middleware calls are never concurrent. Evidence: single `session` binding used through `&mut` at L131, L150, L170, L234; `&mut self` at `middleware/mod.rs:54`, `:65`.
7. After any timeout fires the session is not used again. Evidence: `?` at L135, L154, L175, L238 inside the async block that owns the loop (L129-L277).
8. Once a `Session` exists, its cancellation key is detached on every exit from the block. Evidence: L125-L128, L39-L45.
9. In the capacity retry, at least one backend forwarding precedes every frontend retry. Evidence: L150-L168 precede L170.
10. The drain is entered only immediately after a forwarded `FrontendExpanded`, `Query` or `Sync`, and only when a flag is set at that moment. Evidence: L214-L220.
11. The drain exits only by: both flags clear (L258-L262), a Copy-in/both response (L241-L245), an error (L253-L256), or a timeout (L238). Evidence: those lines; there is no other `break` or `return` in L233-L263.
12. A middleware error ends the session with the middleware's own `Error` value; every other pg-proto error ends it as `InvalidData`. Evidence: L164-L167, L204-L207, L211-L212, L253-L256.

## 6. Assumptions and where each is established

| # | Assumption | Established by |
| --- | --- | --- |
| A1 | pg-proto's `accept` enforces the selected `ServerTlsPolicy` (in particular rejects a non-TLS startup under `Required`) before authentication | nothing found (pg-proto source not on disk) |
| A2 | `StaticMd5ServerCredentials` rejects a wrong password and a wrong user name before `accept` returns `Session` | nothing found |
| A3 | `ClientTlsPolicy::libpq(VerifyFull, ..)` verifies chain and server name against `UpstreamTls` | nothing found |
| A4 | `SslMode::Prefer` behaves as the comment at L359-L360 says; whether it validates the certificate at all | nothing found |
| A5 | A `ForwardThenFlush` middleware output is reported as `ForwardedMessage::FrontendExpanded` (or `FrontendForwarding::Expanded`) | nothing found; asserted only by the comment at L222 and the match at L216 |
| A6 | After `Capacity`, pg-proto retains the rejected frontend message and `forward_frontend` retries exactly that message | nothing found; asserted by the comment at L143-L146 |
| A7 | The middleware's `frontend_operation` side effects happen exactly once per client message across a `Capacity` rejection and its retries | nothing found |
| A8 | `forward_backend` never reads from or blocks on the client socket, and `forward_frontend` never consumes a backend message | nothing found |
| A9 | Every `CommandComplete`, `ErrorResponse` and `ReadyForQuery` that answers a tracked operation is delivered to `backend_operation` with `Some(operation)` that orders correctly against the frontend-assigned ids | nothing found in this file; the three early returns at `context/mod.rs:1573-1575`, `context/settings_protocol.rs:337-339`, `:458` are where a `None` leaves a flag set |
| A10 | A backend that has been sent a flushed Execute, a Query or a Sync always emits a message that clears the flags, or a Copy-in/both response | PostgreSQL protocol behaviour; not checked here. Bound only by L236-L238 when a timeout is configured |
| A11 | No frontend message that depends on a pending outcome is intercepted between a Copy early exit and the outcome's arrival | nothing found; L241-L245 returns to the primary loop with flags set |
| A12 | The cancellation key is not registered before `accept` returns `Session`, or pg-proto removes it itself when `accept` fails or is dropped | nothing found; the guard exists only from L125 |
| A13 | pg-proto compares the CancelRequest secret before forwarding and does not disclose whether a process id exists | nothing found |
| A14 | `BoundedPipeline::new(256)` cannot fail | L109 literal; the `expect` message names the only failure it anticipates. Not verifiable without source |
| A15 | The handler's `context` and the middleware's contexts share state | `context/mod.rs:57-70` (`Arc` fields), `middleware/mod.rs:32`, `:39-40` |
| A16 | `connection_timeout` is acceptable as an idle bound for sessions; a client idle for the duration is disconnected without a protocol message from this file | L133-L135; `main.rs:129-131` logs only |
| A17 | TLS key and certificate files remain readable and unchanged in meaning for the life of the process | nothing found; re-read per connection at L321 via `tls/mod.rs:56`, `:67` |
| A18 | `FrontendMessage::Terminate` always surfaces as `Forwarded(Terminate)` so the loop can end cleanly | `middleware/frontend.rs:176-178` for the middleware half; pg-proto half nothing found |

## 7. Shared state touched

- `CANCELLATION_REGISTRY` (L25-L26): cloned into the intermediary (L108) and the guard (L126); written by pg-proto and by the guard's `Drop` (L42).
- `ConnectionProtocolState.settings_protocol.pending`: read through `settings_in_flight` (L220, L259); written only by the middleware.
- `SchemaMiddleware.in_flight_ddl`: read through `schema_ddl_in_flight` (L220, L258); written only by the middleware.
- No state is written directly by `handler`. Every mutation happens inside `session.forward_*` through the middleware.

## 8. Open questions

1. Unclear; need to inspect pg-proto 0.12.0 `Session::forward_next` to learn the direction selection order and whether a client message already buffered can be serviced before an available backend message. This decides what the Frontend can observe after the Copy early exit at L241-L245 and after a DDL-only drain exit before `ReadyForQuery`.
2. Unclear; need to inspect pg-proto 0.12.0 to confirm that `FrontendMiddlewareOutput::ForwardThenFlush` yields `ForwardedMessage::FrontendExpanded`. If it yields `Frontend(Execute)`, L216 never matches for the flushed-Execute path.
3. Unclear; need to inspect pg-proto 0.12.0 capacity handling: whether `frontend_operation` runs before the `Capacity` check, whether the retained message is the client's original or the middleware's output, and whether `forward_frontend` re-invokes the middleware. `Frontend::intercept` is not idempotent (it increments `in_flight_ddl`, inserts `pending`, allocates metrics scopes).
4. Unclear; need to inspect pg-proto 0.12.0 to learn which frontend messages consume pipeline capacity, in particular `CopyData`. The capacity loop (L149-L209) has no Copy exit, unlike the drain.
5. Unclear; need to inspect pg-proto 0.12.0 operation-id assignment for backend messages: when `backend_operation` receives `None` for `CommandComplete`, `ErrorResponse` or `ReadyForQuery`, and what id a `ReadyForQuery` carries for a simple Query versus a Sync.
6. Unclear; need to inspect pg-proto 0.12.0 `Intermediary::accept` to learn when the cancel key is registered relative to the points where `accept` can fail or be dropped by the timeout at L115-L117.
7. Unclear; need to inspect pg-proto 0.12.0 `StaticMd5ServerCredentials` and startup handling: whether the client-supplied `user` is compared, and which startup parameters (`database`, `options`, `application_name`, replication) are passed upstream alongside `StaticClientCredentials`.
8. Unclear; need to inspect pg-proto 0.12.0 `ClientTlsPolicy::libpq` for the certificate and hostname checks performed under `SslMode::Prefer`, and for whether a failed TLS handshake under `Prefer` falls back to plaintext or only a server `N` reply does.
9. Unclear; need to inspect pg-proto 0.12.0 `BackendForwarding::Forwarded(message)` to learn whether `message` is the upstream original or the middleware replacement, which is what `backend_needs_frontend` (L365-L375) classifies.
10. Unclear; need to inspect `SettingsCommand::may_change_effective` (`context/settings.rs`) and `Intent::changes_schema` (`proxy/schema/middleware.rs`) for statements that alter settings or schema as a side effect without being classified as a barrier or as schema-changing (the drain's coverage is exactly the set those two predicates return true for).
11. Unclear; need to inspect whether any statement shape lets a barrier `Pending` entry with `until_ready: false` end with `EmptyQueryResponse` or `PortalSuspended`, which do not call `settings_execution_succeeded` (`context/lifecycle.rs:453-455`) and so leave the entry until the next `ready`.
12. Unclear; need to inspect what a client observes on `Error::ConnectionTimeout` (L135): no line in `driver.rs` or `main.rs:129-131` sends an `ErrorResponse`, and `diagnostics::connection_timeout` (`postgresql/diagnostics.rs:37-39`) has no call site in this function.
13. Unclear; need to inspect whether `Error::ConnectionClosed` and `Error::CancelRequest` (matched at `main.rs:123-128`) are still constructed on any path reachable from `handler`, given that non-middleware errors are flattened at L351-L353.
