# Frontend::encrypt_literals

- File: `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`
- Lines: L631-L711
- Signature: `async fn encrypt_literals(&mut self, session_id: SessionId, typed_statement: &TypeCheckedStatement<'_>, literal_columns: &Vec<Option<Column>>) -> Result<Vec<Option<EqlOutput>>, Error>`
- Paired helper analysed here: `literals_to_plaintext_skipping` (frontend.rs L1549-L1601).

Note on format: `ANALYSIS_FORMAT.md` was not found in the workspace (the `{baseDir}` placeholder was unresolved), so this file uses purpose, inputs, block-by-block walk, callees, invariants, assumptions, open questions.

Path abbreviations used below:

- `frontend.rs` = `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`
- `inbound_eql.rs` = `packages/cipherstash-proxy/src/postgresql/inbound_eql.rs`
- `context/mod.rs` = `packages/cipherstash-proxy/src/postgresql/context/mod.rs`
- `column_mapper.rs` = `packages/cipherstash-proxy/src/postgresql/column_mapper.rs`
- `from_sql.rs` = `packages/cipherstash-proxy/src/postgresql/data/from_sql.rs`
- `zerokms.rs` = `packages/cipherstash-proxy/src/proxy/zerokms/zerokms.rs`
- `tcs.rs` = `packages/eql-mapper/src/type_checked_statement.rs`

## 1. Purpose

Produces, for one type-checked statement, a positional vector with one slot per EQL-typed literal.
Each slot is either `None` or the `EqlOutput` that replaces that literal in the rewritten SQL.
A slot value comes from one of two sources: Proxy encrypts the literal as plaintext, or the literal text is itself an application-generated EQL payload ("inbound EQL") that is validated and passed through.
The result is consumed by `transform_statement` (frontend.rs L718-L764), which keys each `Some` slot by AST node and hands the map to `TypeCheckedStatement::transform`.

## 2. Callers and reachability

- `query_handler`, simple protocol: frontend.rs L500-L506, once per `Route::Map` statement in a batch, only when `typed_statement.requires_transform()` is true (L492).
- `parse_handler`, extended protocol: frontend.rs L845-L847, only when `typed_statement.requires_transform()` is true (L839).

Both callers pass `literal_columns` from the `Statement` built by `to_encryptable_statement` for the same `typed_statement` (frontend.rs L916, L953, L1275-L1279).
Both are reached from `Frontend::intercept` by an authenticated client; the literal text is client-controlled SQL.

When `requires_transform()` is false the caller never calls this function and forwards the original statement (frontend.rs L524-L526 for Query; L839-L870 leaves `message.query` untouched for Parse).
`requires_transform` is a dry run of the rule tuple with a dummy replacement for every entry of `literals` (tcs.rs L150-L155, L265-L275).

## 3. Inputs and who controls them

| Input | Origin | Controlled by |
|---|---|---|
| `typed_statement.literals` (`Vec<(EqlTerm, &ast::Value)>`) | `EqlMapper::literal_types`, every non-placeholder `ast::Value` whose resolved type is `Value::Eql` (`packages/eql-mapper/src/eql_mapper.rs` L355-L378) | Literal text: client. Which literals are EQL-typed and with which `EqlTerm`: eql-mapper against the pinned schema. |
| `typed_statement.query_operands`, `json_value_selectors`, `json_accessor_paths` | eql-mapper inference (`packages/eql-mapper/src/inference/mod.rs` L163-L207) | Derived from client SQL shape. |
| `literal_columns` | `ColumnMapper::get_literal_columns` (column_mapper.rs L133-L156) over the connection's encrypt config | Catalog content plus client SQL shape. |
| `session_id` | `begin_parse` (frontend.rs L810-L812) or the Query handler's session | Proxy. |
| Effective keyset | `Context::keyset_identifier` reads `state.settings.keyset()` (context/mod.rs L1638-L1644) | Client via `SET CIPHERSTASH.KEYSET_*`, plus operator default. |

## 4. Block-by-block

### 4.1 Empty short-circuit (L637-L644)

`literal_values()` returns `&self.literals` (tcs.rs L261-L263).
An empty list returns `Ok(vec![])` with no metric, metadata, or timing writes.

### 4.2 Inbound EQL detection (L646-L660)

`literal_values.iter().zip(literal_columns)` pairs literal `i` with column `i`.
`zip` stops at the shorter input; there is no length comparison in this function.

For each pair (L650-L652): if the column is `None`, or `(*literal).clone().into_string()` returns `None`, the slot is `None` (not inbound).
`into_string` is a method on `sqltk::parser::ast::Value` in the external `sqltk-parser` crate (0.56.0-cipherstash.3 per `Cargo.lock` L4601-L4602); its source was not located.

Otherwise `inbound_eql::parse(value.as_bytes(), column, literal_is_query_operand(...))` runs (L653-L658).
Any `Err` aborts the whole function via `collect::<Result<..>>()?` at L660.

`literal_is_query_operand` (frontend.rs L1538-L1547) is true when `column.eql_term` is `JsonAccessor` or `JsonPath`, or when `typed_statement.query_operands` contains the literal's `NodeKey` (`packages/eql-mapper/src/query_operands.rs` L54-L56).

`inbound_eql::parse` (inbound_eql.rs L31-L84), all paths:

1. L36-L48: query operand, and term is `JsonAccessor`/`JsonPath`, and the bytes are exactly 32 characters of `[0-9a-f]` (L87-L92).
   Returns `Some(InboundEql::Query(Selector(text)))` after `validate_query_metadata`, which for a selector requires the column to have a SteVec index and repeats the shape test (L168-L178).
   The file's own comment states that a plaintext selector matching this format is treated as an application-generated operand (L9-L14).
2. L50-L55: the bytes are not JSON, or not a JSON object, returns `Ok(None)` (plaintext).
3. L57-L65: object has `v` and `i` and one of `c`, `h`, `sv`.
   Deserialised as `EqlCiphertext`; a deserialisation failure is `Err(InvalidInboundEqlPayload)` (L61-L62), not a fallback to plaintext.
   `validate_storage_metadata` (L94-L125) requires version 3, `identifier == column.identifier`, the record descriptor equal to `"{table}/{column}"`, and SEM-term presence exactly matching the column's configured indexes (L263-L288) or, for SteVec, a SteVec index and a non-empty `sv`.
   Returns `Some(InboundEql::Store(..))`.
   This branch does not consult `query_operand`.
4. L67-L75: object has neither the scalar query shape (`v`, `i`, and one of `hm`/`bf`/`ob`/`op`) nor the SteVec query shape (exactly one key, `sv`), returns `Ok(None)` (plaintext).
5. L76-L78: query-shaped and `query_operand` is false, returns `Err(InvalidInboundEqlPayload)`.
6. L80-L83: query-shaped in a query position; deserialised as `EqlQueryPayload`, then `validate_query_metadata` (L127-L180).
   That check is structural: version, identifier, term presence against configured indexes, and compatibility of `column.eql_term` with the payload kind.
   Returns `Some(InboundEql::Query(..))`.

Consequence recorded as structure: the plaintext-versus-inbound classification is decided by the content of the client's literal (keys present in a JSON object, or a 32-hex string), not by any out-of-band marker.

### 4.3 Plaintext conversion (L661-L662)

`skip[i]` is `inbound[i].is_some()` (L661).
`literals_to_plaintext_skipping` (L1549-L1601) re-reads `typed_statement.literal_values()` (L1554) and zips with `literal_columns` again (L1556-L1558).
Per index:

- `skip.get(index).copied().unwrap_or(false)` true: `Ok(None)` (L1561-L1563). A missing `skip` entry defaults to "do not skip".
- Column `None`: `Ok(None)` (L1596).
- Column `Some`, dispatch on `eql_term.variant()` taken from the *literal's* `EqlTerm` (L1566):
  - `JsonValueSelector`: `json_value_selector_literal_plaintext` (L1611-L1643). Requires a recorded source whose segments are all literals, else `Err(CouldNotParseParameter)` (L1629-L1636). `literal_json_value` returns `None` only for `Value::Null` (from_sql.rs L205), giving `Ok(None)` (L1638-L1640). Otherwise builds `{"path","value"}` and rejects object/array values (from_sql.rs L181-L189).
  - `JsonAccessor` with a recorded chain: `json_accessor_path_literal_plaintext` (L1657-L1686); any `Param` segment is `Err(CouldNotParseParameter)` (L1675-L1683).
  - Everything else: `literal_from_sql(val, col.eql_term(), col.cast_type())` (L1582), which takes the term variant from the *column* rather than from the literal.

The dispatch at L1566 uses the literal's `EqlTerm`; L1582 uses `col.eql_term`.
These are equal because `get_column` sets `Column.eql_term = eql_term.variant()` from the same literal entry (column_mapper.rs L181-L187), provided index alignment holds (section 6, A1).

`literal_from_sql` (from_sql.rs L42-L107):

- String variants go to `text_from_sql` (L50-L65).
- `Value::Boolean` becomes `Plaintext::Boolean` with no check against `col_type` (L68).
- `Value::Null` becomes `None` (L73).
- `HexStringLiteral`, `SingleQuotedByteStringLiteral`, `DoubleQuotedByteStringLiteral` become a text `Plaintext` with no check against `col_type` (L76-L78).
- `Value::Number` goes to `to_f64` for `JsonOrd`, else `decimal_from_sql` (L84-L97).
- `Value::Placeholder` is `Err(Internal)` (L99-L103).

`text_from_sql` (from_sql.rs L336-L416) matches on `(eql_term, col_type)`.
The arm `(Full | Partial, ColumnType::Timestamp)` is `unimplemented!("Timestamp")` (L381-L383), which panics instead of returning `Err`.
Unmatched pairs return `Err(UnsupportedParameterType)` (L411-L414).

Any conversion error is replaced by `MappingError::InvalidParameter(Box::new(col))` (L1585-L1594); the underlying error is kept only in a `debug!` line.
That `debug!` line records `value = ?val`, the client's literal (L1586-L1592).
`literal_from_sql` and `text_from_sql` also emit the literal at debug level (from_sql.rs L47, L341).

### 4.4 Encryption (L664-L672)

`self.context.encrypt(plaintexts, literal_columns)` (context/mod.rs L1647-L1675):

- All plaintexts `None`: returns a same-length vector of `None` at L1652-L1656, before the keyset is read and without calling the encryption service.
- Otherwise reads the keyset once (L1658), calls `self.encryption.encrypt` (L1661-L1664), and rejects a result whose length differs from the input length (L1667-L1673).

On `Err` the function increments `ENCRYPTION_ERROR_TOTAL` (L670-L672) and returns.
That counter is not incremented for failures in 4.2, 4.3, or 4.5.

Production `EncryptionService` is `ZeroKms` (`packages/cipherstash-proxy/src/proxy/mod.rs` L134-L146).
`ZeroKms::encrypt` (zerokms.rs L240-L361):

- Requires a keyset or a configured default (L249-L251).
- Obtains a cipher from the process-wide cache keyed by keyset string (L253, L147-L166).
- Encrypts only positions where both plaintext and column are `Some` (L259-L260).
- Chooses `EqlOperation` from `col.eql_term` (L262-L326): `Store` for `Full`/`Partial`/`Tokenized`; a `Query` op for the four JSON variants when the column has a SteVec index, and `Store` as the `.unwrap_or` fallback when it has none (L277, L288, L299, L316); `JsonExtracted` is `Err(JsonExtractedIsNotAnOperand)` (L323-L325).
- Calls external `encrypt_eql_v3` (L354) and places results back by index with a count check (L360, L497-L514).

### 4.5 Inbound merge (L674-L675)

`merge_inbound_eql(&mut encrypted, inbound, literal_columns)` (frontend.rs L1156-L1241):

- `Query` payloads are written straight into `encrypted[index]` (L1165-L1167). No cryptographic check is applied; the doc comment at L1152-L1155 says these are accepted after the structural validation in `inbound_eql::parse` only.
- `Store` payloads are collected with their column (L1168-L1173).
- Collected payloads are decrypted via `Context::decrypt_inbound_eql` (L1185-L1197), which reads the keyset again (context/mod.rs L1689) and in `ZeroKms::decrypt_eql` rejects any payload whose record `keyset_id` differs from the scoped cipher's keyset (zerokms.rs L401-L408), then authenticates every SteVec entry (L427-L439, L472-L483).
- The recovered plaintexts are re-encrypted for the same columns via `Context::encrypt` (L1207-L1219), a third keyset read (context/mod.rs L1658).
- Each derived value must be `EqlOutput::Store` (L1222-L1229) and `sem_terms_match` must hold (L1230-L1237; inbound_eql.rs L195-L221). Only then is the client's own payload written into `encrypted[index]` (L1238).

The verification columns are clones of the literal's column, including its `eql_term` (L1203-L1206).
For a column whose `eql_term` maps to an `EqlOperation::Query` in `ZeroKms::encrypt`, the derived value is not `Store` and L1222-L1229 returns `Err`.

Indexing: `encrypted[index]` at L1166 and L1238 uses `index` from `inbound.into_iter().zip(columns).enumerate()` (L1163).
`inbound.len()` and `encrypted.len()` both equal `min(literals.len(), literal_columns.len())` (L646-L660; L1556-L1599 with context/mod.rs L1652-L1673), so the index is in range.

### 4.6 Query-operand projection (L677-L686)

For each position, `literal_is_query_operand` is recomputed with the same inputs as at L656.
`project_query_operand` (L1522-L1533) replaces `Some(EqlOutput::Store(c))` with `Some(EqlOutput::Query(c.into_query_operand()))` when the flag is true and leaves every other value unchanged.
`into_query_operand` is in external `cipherstash-client` 0.42.2; source not located.

This applies equally to Proxy-encrypted values and to verified inbound `Store` payloads.
Inbound `Query` payloads are already `EqlOutput::Query` and pass through unchanged.

### 4.7 Logging, timing, metrics (L688-L708)

- `debug!` at L688-L692 emits `literal_columns` and the full `encrypted` vector (output payloads, not plaintext).
- `duration` is measured from L664, so it covers 4.4-4.6 and excludes 4.2-4.3.
- `add_encrypt_duration` and `update_statement_metadata` both go through `with_statement_metrics_scope_mut` (context/mod.rs L1858-L1874), which takes the `protocol_state` write lock and does nothing when `session_id` has no scope (L1870-L1873). The only error is a poisoned lock.
- `m.encrypted = true` is set unconditionally at L702, including when `encrypted_count` is 0.
- The three Prometheus calls at L706-L708 are unconditional. The Bind path gates the same three on `prometheus_enabled()` (frontend.rs L1143-L1147).

### 4.8 Return and downstream contract (L710)

Returns `encrypted`, one slot per zipped pair.
`transform_statement` zips `typed_statement.literals` with this vector (frontend.rs L743-L747), drops `None` slots, and `TypeCheckedStatement::transform` requires the number of supplied replacements to equal the number of non-`Value::Null` literals and every key to be a known literal node (tcs.rs L277-L297).
`SubstituteEncryptedLiterals` replaces only nodes that sit directly under `Expr::Value` (`packages/eql-mapper/src/transformation_rules/substitute_encrypted_literals.rs` L39-L52) and has a postcondition that no replacement is left unused (L64-L73).

A `Selector` slot is emitted as a bare single-quoted string; every other slot is JSON-serialised into a single-quoted string (frontend.rs L732-L735, L1688-L1693).

## 5. Invariants

- I1. `inbound.len() == skip.len() == plaintexts.len() == encrypted.len() == min(literals.len(), literal_columns.len())`. Evidence: L646-L661, L1556-L1599, context/mod.rs L1652-L1656 and L1667-L1673.
- I2. A slot is `Some` on return only if (a) the service returned `Some` for a `Some` plaintext with a `Some` column (zerokms.rs L259-L260, L509-L512), or (b) an inbound payload passed `inbound_eql::parse` and, for `Store`, the decrypt/re-encrypt/compare sequence (L1165-L1167, L1221-L1238).
- I3. A literal recognised as inbound EQL is never also encrypted as plaintext: `skip` forces its plaintext to `None` (L661, L1561-L1563) and the service skips `None` plaintexts (zerokms.rs L260).
- I4. An inbound `Store` payload is accepted only when its identifier, descriptor, and term presence match the destination column (inbound_eql.rs L98-L124), its record keyset equals the active cipher's keyset (zerokms.rs L401-L408), it decrypts (L1185-L1197), and all SEM terms equal an independent re-encryption (L1230-L1237).
- I5. An inbound query-only payload outside a query position is rejected (inbound_eql.rs L76-L78).
- I6. No connection state is written before L697; every earlier failure exits through `?` having touched only `ENCRYPTION_ERROR_TOTAL` (L670-L672) and the ZeroKMS cipher cache (zerokms.rs L200-L202).
- I7. A slot in a query position never leaves this function as `EqlOutput::Store` (L677-L686, L1527-L1530).
- I8. Downstream, a non-`Null` EQL literal left as `None` fails the rewrite rather than being forwarded: `check_all_encrypted_literals_provided` (tcs.rs L281-L287), invoked at tcs.rs L168.

## 6. Assumptions

- A1. `literal_columns[i]` describes `literals[i]`, and both have the same length. Established by `ColumnMapper::get_literal_columns` iterating the same `literals` vector in order (column_mapper.rs L139-L153). That function pushes only when `col.is_some()` (L150-L152), which would shift later indices; the shift does not occur because `get_column` returns `Ok(Some)` or `Err` and never `Ok(None)` (L166-L200). Nothing in `encrypt_literals` checks the lengths; the three `zip` calls (L646-L648, L677-L680, L1556-L1558) truncate silently.
- A2. `literal_columns` was derived from the same `typed_statement` and the same encrypt-config snapshot. Established by `route` building both in one call (frontend.rs L1274-L1279) and both callers passing the pair unchanged (L500-L505, L846).
- A3. `Value::into_string` returns `Some` for exactly the literal forms whose text can carry an inbound payload. Established by: nothing found (external source not located).
- A4. The keyset used to encrypt, to authenticate inbound payloads, and to re-derive SEM terms is the same. Each is a separate read of `protocol_state` (context/mod.rs L1658, L1689, L1658 again via frontend.rs L1209) across `.await` points. Established by: nothing found in this function; depends on whether any other task can write `settings` during the awaits.
- A5. The keyset effective when this function runs is the keyset the statement executes under. For Parse the result is baked into the prepared statement text (frontend.rs L867-L868) and reused by later Bind/Execute. Established outside this function: `finish_parse` records `sensitive` when any literal column is `Some` and passes it to `settings_protocol.capture_literals` (`packages/cipherstash-proxy/src/postgresql/context/lifecycle.rs` L196-L204); for Query, the fail-closed check at frontend.rs L415-L419. Neither was traced to its enforcement point here.
- A6. A client-supplied query-only payload carries SEM terms that are acceptable for the column. Established structurally only by `validate_query_metadata` (inbound_eql.rs L127-L180); nothing checks the term values, and when every literal is an inbound query payload or NULL no keyset is read and no ZeroKMS call is made (context/mod.rs L1652-L1656).
- A7. A 32-character lowercase-hex literal in a JSON selector position is an already-tokenised selector and not a plaintext field name. Established by: nothing found; inbound_eql.rs L9-L14 documents the ambiguity as intentional.
- A8. A JSON object literal containing `v`, `i`, and one of `c`/`h`/`sv` is an EQL storage payload and not plaintext data. Established by: nothing found; such a literal that fails `EqlCiphertext` deserialisation is rejected (inbound_eql.rs L57-L65).
- A9. `encrypt_eql_v3` returns outputs in input order, of the shape implied by the `EqlOperation`, and rejects a `Plaintext` variant that does not match the column's `cast_type` (relevant to from_sql.rs L68 and L76-L78). Established by: only the count check at zerokms.rs L502-L508; ordering and shape are taken on faith.
- A10. `EqlCiphertext::into_query_operand` removes the decryptable ciphertext and keeps the SEM terms. Established by: nothing found (external source not located); the doc comment at frontend.rs L1511-L1521 states the intent.
- A11. `ColumnType::Timestamp` never reaches `text_from_sql` with a `Full`/`Partial` term. Established by: nothing found; the arm panics (from_sql.rs L381-L383).
- A12. A JSON term variant (`JsonAccessor`, `JsonPath`, `JsonOrd`, `JsonValueSelector`) is only assigned to literals on columns that have a SteVec index. Established by: nothing found in Proxy; without the index `ZeroKms::encrypt` falls back to `EqlOperation::Store` (zerokms.rs L277, L288, L299, L316).
- A13. Every EQL-typed literal sits directly under an `Expr::Value` node so the substitution rule can reach it. Established by the rule's postcondition if it is invoked (substitute_encrypted_literals.rs L64-L73); the invocation site was not read.
- A14. `session_id` names a live metrics scope. Not enforced: a missing scope is a silent no-op (context/mod.rs L1870-L1873).

## 7. Calls

| Callee | Kind | What the caller depends on |
|---|---|---|
| `TypeCheckedStatement::literal_values` (tcs.rs L261) | internal | Returns the same ordered `literals` vector on every call; called twice (L637, L1554). |
| `ast::Value::into_string` | external, source not located | Decides which literals are eligible for inbound-EQL detection. |
| `literal_is_query_operand` (L1538) | internal | Role flag; called at L656 and L684 with identical inputs. |
| `QueryOperands::contains_literal` (query_operands.rs L54) | internal | Node-identity membership; populated by eql-mapper inference. |
| `inbound_eql::parse` (inbound_eql.rs L31) | internal | Sole classifier of plaintext versus inbound payload; structural validation of both kinds. |
| `literals_to_plaintext_skipping` (L1549) | internal | Same-length positional plaintexts; `None` for skipped, NULL, and uncolumned literals. |
| `literal_from_sql` / `text_from_sql` (from_sql.rs L42, L336) | internal | Type-directed conversion; contains a panicking arm (L381-L383). |
| `json_value_selector_literal_plaintext`, `json_accessor_path_literal_plaintext` (L1611, L1657) | internal | Refuse when any path segment is a param. |
| `Context::encrypt` (context/mod.rs L1647) | internal | Result length equals input length; all-`None` short-circuit; single keyset read. |
| `ZeroKms::encrypt` (zerokms.rs L240) | internal | Per-position operation selection from `col.eql_term`; positional placement. |
| `encrypt_eql_v3` | external, source not located | Order, shape, and type checking of outputs. |
| `merge_inbound_eql` (L1156) | internal | Authentication and SEM verification of `Store` payloads; direct insertion of `Query` payloads. |
| `Context::decrypt_inbound_eql` / `ZeroKms::decrypt_eql` (context/mod.rs L1685, zerokms.rs L388) | internal | Keyset equality check and all-entry authentication. |
| `inbound_eql::sem_terms_match` (inbound_eql.rs L195) | internal | Equality of every SEM term with an independent derivation. |
| `project_query_operand` (L1522) | internal | Converts `Store` to `Query` in query positions. |
| `EqlCiphertext::into_query_operand` | external, source not located | Strips ciphertext from a stored payload. |
| `Context::add_encrypt_duration`, `Context::update_statement_metadata` (context/mod.rs L1888, L1910) | internal | Write lock on `protocol_state`; no-op for an unknown scope. |
| `counter!` / `histogram!` | external | Process-global recorder. |

## 8. Shared state touched

- Read: `ConnectionProtocolState.settings` keyset (context/mod.rs L1638-L1644), up to three times per call.
- Write: `ConnectionProtocolState.statement_metrics[session_id]` phase timing and metadata (context/mod.rs L1866-L1873).
- Read/write: process-wide `ZeroKms.cipher_cache` (zerokms.rs L162, L200-L202).
- Write: Prometheus `ENCRYPTION_ERROR_TOTAL`, `ENCRYPTION_REQUESTS_TOTAL`, `ENCRYPTED_VALUES_TOTAL`, `ENCRYPTION_DURATION_SECONDS` (L671, L706-L708).
- Outbound: ZeroKMS, for encryption, inbound decryption, and verification re-encryption.

## 9. Open questions

- Unclear which `ast::Value` variants `into_string` returns `Some` for; need to inspect `sqltk-parser` 0.56.0-cipherstash.3 `ast/value.rs`.
- Unclear whether `into_query_operand` removes every decryptable component for both `Encrypted` and `SteVec` payloads; need to inspect `cipherstash-client` 0.42.2 `eql`.
- Unclear whether `encrypt_eql_v3` rejects a `Plaintext` variant that mismatches `cast_type`; need to inspect `cipherstash-client` 0.42.2.
- Unclear whether Backend can write `settings` (keyset) while Frontend is suspended at L669, L1188, or L1210; need to inspect the driver's task structure in `postgresql/driver.rs` and `ConnectionSettings::command_succeeded` call sites.
- Unclear how `capture_literals(&name, sensitive)` constrains a keyset change between Parse and Execute; need to inspect `postgresql/context/settings_protocol.rs`.
- Unclear whether `ColumnType::Timestamp` can be produced for an encrypted column; need to inspect `proxy/encrypt_config/from_domain.rs`.
- Unclear whether eql-mapper can assign a JSON term variant to a literal on a column lacking a SteVec index; need to inspect the trait bounds in eql-mapper inference and `proxy/schema/eql_domains.rs`.
- Unclear whether `check_postcondition` of `SubstituteEncryptedLiterals` is invoked on the real run; need to inspect `DryRunnable` and the tuple `TransformationRule` impl in `packages/eql-mapper/src/transformation_rules/`.
- Unclear whether `set_encrypted_values_count` accumulates or overwrites across the statements of one simple-query batch that share a `session_id`; need to inspect `StatementMetadata`.
- Unclear what `decimal_from_sql` does for a `Value::Number` against each `ColumnType`; need to inspect from_sql.rs beyond L260.
