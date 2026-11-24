## `Frontend::bind_handler` in packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs (L1003-L1080)

File abbreviations used in citations below.

- `FE` = `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`
- `LC` = `packages/cipherstash-proxy/src/postgresql/context/lifecycle.rs`
- `CTX` = `packages/cipherstash-proxy/src/postgresql/context/mod.rs`
- `SP` = `packages/cipherstash-proxy/src/postgresql/context/settings_protocol.rs`
- `PS` = `packages/cipherstash-proxy/src/postgresql/context/prepared_statements.rs`
- `ST` = `packages/cipherstash-proxy/src/postgresql/context/statement.rs`
- `PO` = `packages/cipherstash-proxy/src/postgresql/context/portal.rs`
- `BIND` = `packages/cipherstash-proxy/src/postgresql/rewrite/bind.rs`
- `IEQL` = `packages/cipherstash-proxy/src/postgresql/inbound_eql.rs`
- `SQL` = `packages/cipherstash-proxy/src/postgresql/data/from_sql.rs`
- `ZK` = `packages/cipherstash-proxy/src/proxy/zerokms/zerokms.rs`
- `SM` = `packages/cipherstash-proxy/src/proxy/schema/middleware.rs`
- `DRV` = `packages/cipherstash-proxy/src/postgresql/driver.rs`
- `EM` = `packages/eql-mapper/src/eql_mapper.rs`

**Purpose:**
Local preparation of one extended-protocol Bind.
It records the Bind in the connection's settings journal and schema middleware, decides between an `Encrypted` and a `Passthrough` portal, replaces the bound values of encrypted output params with EQL payloads (or NULL), installs the portal, and tells the caller whether the Bind message must be re-serialized.
It is the only place where client-bound parameter values for encrypted columns are converted before they are written to PostgreSQL.

---

**Inputs & Assumptions:**

- `operation` (`OperationId`): pg-proto's correlation id for this frontend message.
  Trust: trusted (assigned by pg-proto, not by the client).
  It keys `ConnectionProtocolState.operations` (LC L250) and the settings journal (SP L201).
- `bind` (`Bind`, by value, mutable): built by the caller from the wire message with `Bind::try_from` (FE L279, BIND L348-L395).
  Trust: semi-trusted (authenticated client controls every field).
  - `portal`, `prepared_statement`: arbitrary byte names.
  - `param_values`: arbitrary count, arbitrary bytes, each NULL or not (BIND L375-L380).
  - `param_format_codes`: any `i16` other than `1` becomes `Text` (`format_code.rs` L7-L13).
  - `result_columns_format_codes`: arbitrary count, same `i16` mapping (BIND L387-L391).
- Implicit state read:
  - Effective `unsafe_disable_mapping` (LC L239, CTX L1630-L1636).
  - Effective keyset, read later inside `Context::encrypt` / `decrypt_inbound_eql` (CTX L1658, L1689).
  - `prepared_statements[name]` (statement, metrics scope) (LC L249, CTX L1126, CTX L1076-L1082).
  - Settings-protocol `prepared[name]` and `portals[portal]` (SP L200, L210).
  - Schema middleware `prepared[name]` (SM L492-L497).
- Preconditions and what establishes each:
  - Config-level mapping is enabled. Established by `intercept` returning early at FE L189-L211 when `context.mapping_disabled()` is true; `bind_handler` is not reached on that path.
  - The connection is not discarding a failed extended batch. Established by FE L179-L187.
  - No settings-changing or DDL execution issued earlier on this connection is still unconfirmed. Established outside this function by DRV L214-L267 (drain loop after forwarding); not checked inside `bind_handler` or `begin_bind`. See Open Questions.
  - `Bind::try_from` has produced one `BindParam` per wire parameter with a resolved format code. Established by BIND L362-L381. The format-code count check at BIND L363-L373 runs only inside the per-parameter loop, so a Bind with zero parameters and N format codes passes unchecked.

---

**Outputs & Effects:**

- Returns `Ok(None)`: caller forwards the client's original Bind bytes unchanged (FE L213, L285-L287).
- Returns `Ok(Some(FrontendMessage::Bind))`: caller forwards the re-serialized Bind (FE L280-L283, BIND L539-L557).
- Returns `Err`: caller calls `refuse_bind`, sets `failed_extended_batch`, and forwards the **original** client Bind with only the statement name replaced by a random name (FE L288-L320, FE L1745-L1754).
- State writes on every path that enters the function:
  - Settings journal entry for `operation` and removal of the previous settings-protocol portal of that name (SP L200-L209).
  - Schema middleware `portals[portal]` inserted or removed (SM L498-L506).
  - `operations[operation].preparation = Some(Preparation::Bind{..})` (LC L250-L253), unless `begin_bind` failed before that line.
  - A cloned metrics scope when mapping is not disabled and the statement has one (CTX L1120-L1137).
- State writes on success: `install_portal` replaces `portals[name]`, `portal_operations[name]`, drops `suspended_executions[name]` (CTX L188-L214); `preparation` is taken (LC L267-L274).
- External interaction: ZeroKMS via `EncryptionService::encrypt` and `decrypt_inbound_eql` (ZK L240-L361, L377-L385).
- Log emission: the whole `Bind` including parameter bytes at `debug!` under `PROTOCOL` (FE L1038); plaintexts at `debug!` under `MAPPER` (FE L1111); each param at `debug!` under `ENCODING` (SQL L24, L341).
- Metrics: two counters on the mapping-disabled branch (FE L1013-L1014); statement metadata `param_bytes`, `encrypted` (FE L1034-L1036, L1055-L1056).

---

**Block-by-Block:**

```rust
// FE L1008-L1010
let knowledge =
    self.context
        .begin_bind(operation, bind.portal.clone(), &bind.prepared_statement)?;
```
- **What:** Registers the Bind and returns `BindKnowledge { statement, metrics, mapping_disabled }` (LC L254-L258).
- **Why here:** Every later decision reads `knowledge`; the settings refusal (prepared settings differ from current settings) must precede any encryption.
- **Callee order (LC L231-L259):**
  1. `bind_settings_statement` (CTX L1456-L1483) takes the write lock, snapshots `EffectiveSettings`, computes `encrypted_values` from `statement.param_columns.iter().any(Option::is_some)` (CTX L1467-L1470), snapshots the existing portal (CTX L1471), and calls `SettingsProtocol::bind`.
  2. `SettingsProtocol::bind` (SP L191-L231) first removes `portals[portal]` and inserts the journal mutation (SP L200-L209), then returns `Ok(())` without a settings check when `prepared[name]` is absent (SP L210-L212), then returns `Err(refusal)` when `prepared.needs_mapping` and the Parse-time settings differ from current (SP L213-L219), then inserts the `Bound` record (SP L221-L229).
     The keyset comparison applies only when `prepared.literal_sensitive` is true (SP L216, SP L33-L41); a keyset change between Parse and Bind of a statement with encrypted params and no encrypted literals does not refuse here.
  3. `bind_schema_statement` (CTX L1361-L1363, SM L491-L507) inserts or removes the portal's schema intent. It has no error return and no journal.
  4. `unsafe_disable_mapping()` re-reads settings under a new read lock (LC L239, CTX L1630-L1636).
  5. `start_portal_metrics_scope` only when mapping is not disabled (LC L240-L244); returns `Ok(None)` when the name has no metrics scope (CTX L1126-L1128) and `Err(StatementMetricsUnavailable)` when the template id is recorded but its scope is gone (CTX L1129-L1131).
  6. Write lock: reads `prepared_statements.statement(name)` and sets `preparation` (LC L245-L253).
- **Assumes:** The three separate lock acquisitions (steps 1, 4, 6) see the same settings and statement. No `.await` separates them (LC L237-L253 is synchronous).
- **Establishes:** `operations[operation].preparation == Some(Bind{name: portal, metrics})` on `Ok`. On `Err` from step 1, the journal mutation already exists (SP L201) and `preparation` is not set.
- **Depended on by:** `finish_bind` (LC L267-L274) and `refuse_bind` (LC L293-L300).

```rust
// FE L1011-L1018
if knowledge.mapping_disabled {
    warn!(msg = "Encrypted statement mapping is not enabled");
    counter!(STATEMENTS_PASSTHROUGH_MAPPING_DISABLED_TOTAL).increment(1);
    counter!(STATEMENTS_PASSTHROUGH_TOTAL).increment(1);
    self.context
        .finish_bind(operation, Portal::passthrough(None))?;
    return Ok(None);
}
```
- **What:** When `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING` is effective, installs `Portal::Passthrough { session_id: None }` and forwards the Bind untouched.
- **Why here:** It precedes the `is_prepared` check (FE L1023) and ignores `knowledge.statement`, so on this branch neither an unknown statement name nor a stored `Statement` with encrypted params changes the outcome.
- **Assumes:** A statement that carries a `Statement` (mapped at Parse) cannot reach this branch. This is established only inside `SettingsProtocol::bind` at SP L213-L219, and only when (a) `prepared[name]` exists in the settings protocol (SP L210-L212) and (b) `prepared.needs_mapping` is true (SP L213). `needs_mapping` is `eql_mapper::requires_schema` (SP L173), which is a superset of `requires_type_check` (EM L120-L122), and `Route::Map` is produced only after `requires_type_check` (FE L1263-L1281), so (b) holds for every mapped statement. (a) rests on `prepare_parse` calling `prepare_settings_statement` (LC L159) before `finish_parse` stores the `Statement` (LC L199-L203); nothing in `bind_handler` re-checks it.
- **Establishes:** Portal without metrics scope; `Ok(None)`.

```rust
// FE L1023-L1028
if !self.context.is_prepared(&bind.prepared_statement)? {
    return Err(MappingError::UnknownPreparedStatement { .. }.into());
}
```
- **What:** Refuses a Bind whose statement name has no entry in `prepared_statements`.
- **Why here:** `knowledge.statement == None` is otherwise indistinguishable between "Parse routed to forward" and "Proxy never saw a Parse"; this separates them.
- **Callee:** `is_prepared` is `HashMap::contains_key` (CTX L1076-L1082, PS L49-L51). An entry exists once any of `set_statement`, `set_metrics_scope`, `set_parse`, `set_deallocation` ran for the name (PS L57-L83, each `entry().or_default()`); `begin_parse` runs two of them for every Parse (LC L107-L112).
- **Assumes:** Entry presence equals "PostgreSQL holds a statement of this name that Proxy routed". Names prepared by SQL `PREPARE` in a simple Query are not inserted by any code read here, so a Bind to such a name takes the `Err` branch.
- **Ordering fact:** This check runs after `begin_bind` already wrote the settings journal, the schema-middleware portal map, and `preparation`. On `Err` the caller's `refuse_bind` clears `preparation` and stores the error response (LC L283-L304); it does not touch the settings journal or `SM.portals`.

```rust
// FE L1030-L1040
let session_id = knowledge.metrics;
...
let param_bytes: usize = bind.param_values.iter().map(|p| p.bytes.len()).sum();
self.context.with_metrics_scope(session_id, |m| m.metadata.set_param_bytes(param_bytes))?;
debug!(target: PROTOCOL, client_id = self.context.client_id, bind = ?bind);
let mut portal = Portal::passthrough(session_id);
```
- **What:** Records total parameter bytes, logs the Bind, defaults the portal to `Passthrough`.
- **Assumes:** `with_metrics_scope` is a no-op for `None` and for an id with no scope (CTX L1920-L1932, L1870-L1872).
- **Establishes:** Default route is passthrough; only the `has_projection` branch upgrades it.
- **Note:** `Bind` and `BindParam` derive `Debug` (BIND L26, L39), so FE L1038 formats every bound value before any encryption.

```rust
// FE L1042-L1058
if let Some(statement) = knowledge.statement {
    if statement.has_params() {
        let encrypted = self.encrypt_params(session_id, &bind, &statement).await?;
        bind.rewrite(&statement.output_params, encrypted)?;
    }
    if statement.has_projection() {
        portal = Portal::encrypted_with_format_codes(statement, bind.result_columns_format_codes.to_owned(), session_id);
        self.context.with_metrics_scope(session_id, |m| m.metadata.encrypted = true)?;
    }
};
```
- **What:** Encrypts and rewrites params when the statement has client params; chooses the `Encrypted` portal when the statement has a projection.
- **Gate mismatch:** `has_params()` tests `param_columns` (ST L124-L126), while `encrypt_params` and `rewrite` iterate `output_params` (FE L1095-L1109, BIND L261-L298). Parse sets `output_params` from `param_columns` (FE L939-L947) or from the mapper's `ParamPlan` (FE L861-L865). Nothing in `Statement` ties the emptiness of one to the other.
- **Portal choice:** `has_projection()` is `!projection_columns.is_empty()` (ST L128-L130); a projection whose entries are all `None` still yields `Encrypted`. A statement with encrypted params and no projection keeps the `Passthrough` portal.
- **Result format codes:** copied from the client without comparing their count to `projection_columns.len()` (FE L1052). `Portal::format_codes` returns the client's vector as-is when it has two or more entries, regardless of `row_len` (PO L56-L68).
- **Depended on by:** Backend row handling reads the portal installed below; `Portal::format_codes` is `unreachable!()` for `Passthrough` (PO L69-L71).

`encrypt_params` (FE L1089-L1150), step by step:

1. `bind.inbound_eql(&statement.output_params)` (BIND L129-L156).
   For each output param with a column and an `Input` source whose bound value exists and is not NULL, strips one leading `0x01` when the param is binary (BIND L148-L152) and calls `inbound_eql::parse`.
   `JsonValueSelector` and `JsonAccessorPath` sources are never inspected for inbound EQL (BIND L139-L141).
   `parse` (IEQL L31-L84) returns `Ok(None)` for anything not shaped as EQL, `Store` for `{v,i}` plus one of `c|h|sv` after `validate_storage_metadata` (IEQL L57-L65), `Query` for term-only shapes when `query_operand` is true, and `Err` for a term-only shape when `query_operand` is false (IEQL L76-L78).
   A storage-shaped payload is accepted as `Store` irrespective of `query_operand` (IEQL L60-L65).
   A 32-character lowercase-hex value in a `JsonAccessor`/`JsonPath` query position is taken as an already-tokenized selector (IEQL L36-L48, comment IEQL L9-L14).
2. `bind.to_plaintext_skipping(output_params, postgres_param_types, &skip)` (BIND L72-L125).
   Returns `Ok(None)` for skipped positions (BIND L82-L84), native params (BIND L85-L88), and an `Input` index beyond the bound params (BIND L100-L102).
   Every error of `bind_param_from_sql` is replaced with `MappingError::InvalidParameter(col)` (BIND L112-L114).
   The decode type is the client's Parse-time OID for that input index when it resolves, otherwise the column type (BIND L401-L406).
   Text-format values pass through `from_utf8_lossy` first (BIND L532-L537, SQL L31).
   `text_from_sql` has an `unimplemented!("Timestamp")` arm (SQL L381-L383) and `binary_from_sql` has `unimplemented!("TIMESTAMPTZ")` (SQL L506); these are panics, not `Err`. `ColumnType::Timestamp` is produced by `proxy/encrypt_config/from_domain.rs` L79.
   Selector path steps bound as params are read as raw lossy text (BIND L242-L243); any missing or NULL step yields `None` (BIND L244).
3. `self.context.encrypt(plaintexts, &output_param_columns)` (CTX L1647-L1675).
   Returns all-`None` without reading the keyset or calling the service when every plaintext is `None` (CTX L1652-L1656).
   Otherwise reads the keyset under a fresh read lock (CTX L1658) and enforces `encrypted.len() == plaintexts.len()` (CTX L1667-L1673).
4. `merge_inbound_eql` (FE L1156-L1241).
   `Query` payloads are written straight into `encrypted[index]` (FE L1165-L1167); the comment at FE L1153-L1155 states they have no ciphertext to authenticate and rely on `inbound_eql::parse` validation only.
   `Store` payloads are decrypted with `decrypt_inbound_eql` (FE L1185-L1197), re-encrypted for the destination column (FE L1207-L1219), and compared with `sem_terms_match` (FE L1230); the inbound ciphertext is used only if the derived value is `Some(EqlOutput::Store(_))` and matches (FE L1222-L1238).
   `decrypt_inbound_eql` and the second `encrypt` each read the keyset independently (CTX L1689, L1658).
   ZeroKMS-side `decrypt_eql` with `AllEntries` rejects any ciphertext whose keyset id differs from the cipher's (ZK L401-L408).
5. `project_query_operand` for every output (FE L1126-L1128, L1522-L1533): `Store` becomes `Query` via `into_query_operand()` when `output.query_operand`.
6. Metrics: `m.metadata.encrypted = true` is set whenever this function returns `Ok`, including when `encrypted_count == 0` (FE L1133-L1140).

`Bind::rewrite` (BIND L261-L298):

- Positional path when `output_params.len() == param_values.len()` and every output is `Input(idx)` at its own index (BIND L266, ST L138-L143): three-way `zip` over params, outputs, and `encrypted` (BIND L267-L274).
  `rewrite` does not check `encrypted.len()`; equality with `output_params.len()` comes from `to_plaintext_skipping` mapping over `output_params` (BIND L78-L124) and the length check in `Context::encrypt` (CTX L1667).
- Rebuild path otherwise (BIND L278-L297): each output clones `param_values[primary_input]` or fails with `ProtocolError::MissingBoundParameter` (BIND L281-L286); the param list, its format codes, and `reshaped` are replaced (BIND L293-L295).
  A client that binds more values than the statement's positional outputs lands on this path, and the surplus values are absent from the re-serialized Bind.
- `apply_output` (BIND L310-L321): an output with a column and no ciphertext becomes NULL (BIND L315-L318); otherwise `apply_encrypted` writes the selector as bare text (BIND L333-L335) or the JSON-serialized payload, with a leading `0x01` when the param's format code is binary (BIND L337-L340, L457-L467).
  `rewrite_null` does not mark an already-NULL param dirty (BIND L490-L498).

```rust
// FE L1060-L1061
debug!(target: MAPPER, client_id = self.context.client_id, portal = ?portal);
self.context.finish_bind(operation, portal)?;
```
- **What:** Installs the portal.
- **Callee (LC L261-L281):** takes `preparation`; returns `ContextError::UnknownOperation` if it is not `Preparation::Bind` (LC L267-L274); `install_portal` removes any previous portal and suspended execution of that name, records `PortalOwner { operation, deallocation: None }`, inserts the new `Arc<Portal>`, and releases unreferenced metrics scopes (CTX L188-L214).
- **Why here:** It is the last fallible call. Everything after is infallible (`requires_rewrite`, `From<Bind>`), so an `Err` return never coexists with an installed portal for this operation.
- **Establishes:** `portals[bind.portal]` reflects this Bind before PostgreSQL has acknowledged it (speculative; rollback is via the settings journal, CTX L216-L266).
- **Depended on by:** the caller's `bind_statement_effects` (FE L281, L286), which writes `deallocation` into `portal_operations[portal]` (CTX L814-L824); `Context::execute` (LC L306-L315).

```rust
// FE L1063-L1075
if bind.requires_rewrite() {
    let message = FrontendMessage::from(bind);
    debug!(target: MAPPER, ..., msg = "Rewrite Bind", ?message);
    Ok(Some(message))
} else {
    Ok(None)
}
```
- **What:** Re-serializes only when a param is dirty or the list was reshaped (BIND L48-L54).
- **Assumes:** "not dirty" implies the original bytes are safe to forward. This holds for an encrypted output only when its bound value was already NULL (BIND L491-L493), because `apply_output` otherwise always calls `rewrite`, `rewrite_text`, or `rewrite_null`, each of which sets `dirty` (BIND L466, L481, L497).
- **Establishes:** On `Ok(None)` the caller forwards the client's bytes verbatim (FE L213).

---

**Cross-Function Dependencies:**

- Callee `Context::begin_bind` (internal): establishes `preparation`, the settings refusal, the statement snapshot. The settings refusal exists only on the path where `SettingsProtocol.prepared[name]` is present and `needs_mapping` is true (SP L210-L219).
- Callee `Context::is_prepared` (internal): key presence only (PS L49-L51).
- Callee `Context::finish_bind` (internal): atomic portal replacement under one write lock (LC L262-L276).
- Callee `Frontend::encrypt_params` (internal): one `Option<EqlOutput>` per output param, in output order.
- Callee `Bind::inbound_eql` / `inbound_eql::parse` (internal): structural classification of client-supplied EQL; column identifier, descriptor, and term-presence checks (IEQL L94-L125, L127-L180).
- Callee `Bind::to_plaintext_skipping` / `bind_param_from_sql` (internal): typed plaintext or `InvalidParameter`; two panic arms (SQL L382, L506).
- Callee `Context::encrypt` (internal wrapper): length equality (CTX L1667); keyset read at call time (CTX L1658).
- Callee `ZeroKms::encrypt` (internal, calls `cipherstash_client`): zips plaintexts with columns (ZK L259), so a shorter `columns` slice truncates silently; `encrypt_params` passes slices both derived from `output_params` (FE L1097-L1109). `place_ciphertexts` enforces result count against the non-`None` inputs (ZK L497-L514). `MissingKeysetIdentifier` when neither default nor connection keyset exists (ZK L249-L251).
- Callee `cipherstash_client::encrypt_eql_v3`, `ScopedCipher::init`, `ScopedCipher::decrypt` (external-black-box here): assumed to return one output per prepared plaintext in input order (count is checked at ZK L502; order is not checkable).
- Callee `Bind::rewrite` (internal): every output with a column ends as ciphertext or NULL (BIND L315-L320); relies on its caller for `encrypted.len()`.
- Callee `Portal::encrypted_with_format_codes` / `Portal::passthrough` (internal): plain constructors (PO L18-L41).
- Callee `Context::with_metrics_scope` (internal): no-op when the scope is absent.

- Callers: only `Frontend::intercept` (FE L276-L322).
  - `Bind::try_from(bind)?` at FE L279 propagates its error out of `intercept` with `?`; it does not go through `refuse_bind`. `intercept` errors surface as `ForwardError::Middleware` and end the driver loop (DRV L204-L206, L211).
  - On `Ok`, the caller runs `bind_statement_effects` (FE L281, L286).
  - On `Err`, all three match arms do the same thing apart from the log message (FE L289-L319): `refuse_bind`, set `failed_extended_batch`, forward `extended_bind_error(failed_bind, ..)`.
    `extended_bind_error` changes only `bind.statement` (FE L1752); the forwarded message still carries the client's original `parameters` (clone taken at FE L277).
    The caller relies on PostgreSQL rejecting a Bind to the random `__cipherstash_<uuid>` name (FE L1756-L1760) so that the Backend can substitute the stored diagnostic.

- Shared state:
  - `ConnectionProtocolState.operations`, `portals`, `portal_operations`, `suspended_executions`, `statement_metrics`, `settings_protocol` — also written by Parse/Execute/Close lifecycle methods and by Backend acknowledgement handlers (`preparation_acknowledged` LC L408-L420, `restore_failed_preparation` CTX L216-L266, `ready_for_query` CTX L931-L1015).
  - `SchemaMiddleware.portals` — written by `bind` (SM L498-L506), read by `execute` (SM L512).
  - `ZeroKms.cipher_cache` — process-wide, keyed by keyset identifier string or `"default"` (ZK L147-L152, L162, L200-L202).

- Invariant couplings:
  - "Effective settings come only from confirmed executions" (CONTEXT.md L141-L143) couples to this function through three separate reads of settings: SP bind snapshot (CTX L1466), `unsafe_disable_mapping` (LC L239), and keyset at each `encrypt`/`decrypt_inbound_eql` call (CTX L1658, L1689). The reads straddle `.await` points in `encrypt_params` (FE L1118, L1124).
  - The keyset used to encrypt bound values is not stored on the portal; consistency with Execute is checked later by `SettingsProtocol::execute` comparing `Bound.settings` with current settings when `encrypted_values || literal_sensitive` (SP L242-L249). `Bound.settings` is the snapshot from CTX L1466, taken before the keyset reads in `encrypt`.
  - Local preparation is speculative (LC L3-L5): a portal installed here is rolled back only when PostgreSQL's failure reaches `restore_failed_preparation`. `SchemaMiddleware.portals` has no corresponding restore in the code read.

---

**Open Questions:**

- unclear; need to inspect `pg-proto` forwarding and `DRV` L140-L280 to confirm that `ForwardThenFlush` maps to `ForwardedMessage::FrontendExpanded` and that every settings-changing Execute is drained before the next frontend message reaches `intercept`.
- unclear; need to inspect `pg-proto` session scheduling to confirm that `Backend::intercept` cannot mutate `protocol_state.settings` while `bind_handler` is suspended at FE L1046 (the comment at DRV L226-L228 describes a single-task intermediary; no code line read here enforces it).
- unclear; need to inspect `eql-mapper` `ParamPlan` construction to confirm that `output_params` is empty whenever `param_columns` is empty, which FE L1045 relies on.
- unclear; need to inspect `Backend::data_row_handler` and `Portal::format_codes` call sites for how a client-supplied `result_columns_format_codes` length that differs from the row width is handled.
- unclear; need to inspect `Backend` error handling and `restore_failed_preparation` to determine the state of `SchemaMiddleware.portals[portal]` after a Bind refused at FE L1023 or inside the async block, since `SM::bind` already replaced or removed the entry at LC L238.
- unclear; need to inspect paths that remove from `SettingsProtocol.prepared` (`succeeded` SP L299-L300, `failed` SP L407-L415, `close`, `deallocated`) against paths that remove from `PreparedStatements`, to confirm no state leaves a `Statement` present with `SettingsProtocol.prepared[name]` absent (the SP L210-L212 early return).
- unclear; need to inspect the connection task's panic handling (`main.rs` accept loop) for what happens when `text_from_sql`/`binary_from_sql` hit the `unimplemented!` arms at SQL L382 and L506.
- unclear; need to inspect `cipherstash_client::encrypt_eql_v3` for output ordering relative to its input vector, which `place_ciphertexts` (ZK L510-L512) relies on.
- unclear; need to inspect `error_to_response` (`error_handler.rs` L37) for which fields of `MappingError::InvalidParameter(Box<Column>)` and `UnknownPreparedStatement { name }` reach the client diagnostic.
