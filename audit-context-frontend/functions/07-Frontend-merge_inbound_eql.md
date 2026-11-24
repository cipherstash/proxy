# Frontend::merge_inbound_eql

File: `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`, L1156-L1241.
All line numbers without a file name refer to `frontend.rs`.
`inbound_eql.rs` means `packages/cipherstash-proxy/src/postgresql/inbound_eql.rs`.
`context/mod.rs` means `packages/cipherstash-proxy/src/postgresql/context/mod.rs`.
`zerokms.rs` means `packages/cipherstash-proxy/src/proxy/zerokms/zerokms.rs`.
`bind.rs` means `packages/cipherstash-proxy/src/postgresql/rewrite/bind.rs`.

The format and example resources named in the task (`ANALYSIS_FORMAT.md`, `FUNCTION_MICRO_ANALYSIS_EXAMPLE.md`, `DOMAIN_NOTES.md`) were not found in the repository, so this document follows the section layout of `audit-context/functions/01-Frontend-route.md`.

The source of `cipherstash-client` 0.42.2 (pinned at root `Cargo.toml` L50) was not found under `~/.cargo/registry/src`, so every item from that crate is treated as a black box.

## 1. Purpose

`merge_inbound_eql` takes the positional encryption output Proxy has already produced (`encrypted`), a parallel vector of client-supplied EQL payloads that `inbound_eql::parse` classified (`inbound`), and the parallel destination columns (`columns`) (L1156-L1161).
For each position holding a client-supplied payload it overwrites `encrypted[index]` with that payload, in one of two ways:

- `InboundEql::Query`: written directly with no further check in this function (L1165-L1167).
- `InboundEql::Store`: written only after decrypting it, re-encrypting the recovered plaintext for the destination column, and comparing searchable metadata (L1168-L1173, L1181-L1239).

It returns `Ok(())` or `Err(EncryptError::InvalidInboundEqlPayload)`.
No other error variant leaves this function: both callee errors are replaced at L1196 and L1218, and the three local refusals use the same variant (L1170, L1228, L1236).

The function takes `&self`, writes no connection state, and mutates only the caller's `encrypted` slice.
Side effects are `warn!` lines on the `ENCRYPT` target (L1190-L1195, L1212-L1217, L1223-L1227, L1231-L1235) and whatever the encryption service does (cipher cache insert, ZeroKMS requests).

## 2. Inputs and who controls them

| Input | Source | Controlled by |
|---|---|---|
| `inbound` | `inbound_eql::parse` over SQL literal text (L646-L660) or bound parameter bytes (`bind.rs` L129-L156) | Authenticated client; shape-validated by `parse` |
| `columns` | `literal_columns` from the stored statement (L504, L846) or `statement.output_params[*].column` (L1105-L1109) | Type checker output over catalog-derived schema and encrypt config |
| `encrypted` | `Context::encrypt` over the non-inbound plaintexts (L666-L672, L1115-L1121) | Proxy; positions that hold an inbound payload are `None` on entry because the caller passes them as skipped plaintexts (L661-L662, L1096-L1101) |
| Effective keyset | `Context::keyset_identifier` read inside each service call (`context/mod.rs` L1638-L1644, L1658, L1689) | Authenticated client through `SET CIPHERSTASH.KEYSET_*`; operator through `default_keyset_id` (`zerokms.rs` L137, L181) |

## 3. Callers and what they establish first

Three call sites exist: L674, L1123 and the unit test at L2870.

### 3.1 `encrypt_literals`, L674

Reached from `query_handler` (L500-L506) and `parse_handler` (L845-L847), in both cases only when `typed_statement.requires_transform()` (L492, L839).

- `inbound` is built by zipping `literal_values` with `literal_columns` (L646-L648), so `inbound.len() == min(literal_values.len(), literal_columns.len())`.
- A position is `None` when the column is `None` or the literal has no string form (L650-L652).
- Otherwise `inbound_eql::parse(value.as_bytes(), column, literal_is_query_operand(...))` runs and any `Err` fails the whole statement (L653-L660).
- `plaintexts` comes from `literals_to_plaintext_skipping`, which zips the same two sequences (L1556-L1558) and yields `None` for skipped positions (L1561-L1563).
- `encrypted` comes from `Context::encrypt(plaintexts, literal_columns)`; its length equals `plaintexts.len()` on both return paths (`context/mod.rs` L1652-L1656, L1667-L1674).
- After the merge, `project_query_operand` runs on every position with the same query-operand predicate used for `parse` (L677-L686).

### 3.2 `encrypt_params`, L1123

Reached from `bind_handler` when a stored `Statement` exists and `statement.has_params()` (L1042-L1047).

- `inbound` comes from `Bind::inbound_eql(&statement.output_params)`, one entry per output param (`bind.rs` L133-L155).
- An entry is `None` when the output param has no column (`bind.rs` L136-L138), when its source is not `OutputParamSource::Input` (`bind.rs` L139-L141), when the input index has no bound value (`bind.rs` L142-L144), or when the param is NULL (`bind.rs` L145-L147).
- For a binary param whose first byte is `1`, `parse` receives `param.json_bytes()`; otherwise it receives the raw bytes (`bind.rs` L148-L152).
- `output_param_columns` is built from the same `output_params` (L1105-L1109), so `inbound`, `columns` and `encrypted` have equal length.
- After the merge, `project_query_operand(output.query_operand, ...)` runs per position (L1126-L1128), then `bind.rewrite` serialises each `Some` value into the Bind (L1047; `bind.rs` L261-L345).

### 3.3 Unit test, L2867-L2881

Calls the function directly with `InboundEql::Store` and `columns = [None]`, and asserts `InvalidInboundEqlPayload`.
This is the only caller that bypasses `inbound_eql::parse`.

## 4. Block-by-block

### 4.1 Classification loop, L1162-L1176

`inbound.into_iter().zip(columns).enumerate()` (L1163) iterates `min(inbound.len(), columns.len())` positions.
Inbound entries beyond `columns.len()` are dropped without a merge and without an error.
Both production callers supply equal lengths (section 3).

`encrypted[index]` at L1166 is a direct slice index.
It panics when `index >= encrypted.len()`.
Nothing in this function compares `encrypted.len()` with the zip length; the bound is supplied by the callers through `Context::encrypt` (`context/mod.rs` L1667).

`InboundEql::Query` (L1165-L1167): the payload is moved into `encrypted[index]` as `EqlOutput::Query`.
`column` is not read on this arm.
No decrypt, no re-derivation, and no keyset comparison happens for this arm; the doc comment at L1153-L1155 names `inbound_eql::parse` as the only gate.

`InboundEql::Store` (L1168-L1173): a `None` column returns `InvalidInboundEqlPayload` (L1169-L1171); otherwise `(index, ciphertext, column.clone())` is queued in `positions` (L1172).

`None` (L1174): the position keeps whatever `Context::encrypt` produced.

Query positions are written into `encrypted` during this loop, before any Store payload is verified.
When a later step returns `Err`, both callers propagate with `?` and drop `encrypted` (L674-L675, L1123-L1124).

### 4.2 Early return, L1177-L1179

When no Store payload exists the function returns `Ok(())` with no service call.
A statement whose only inbound payloads are Query-shaped therefore makes no decrypt request from this function.

### 4.3 Decrypt, L1181-L1197

Every queued ciphertext is cloned into `Vec<Option<EqlCiphertext>>` with all entries `Some` (L1181-L1184) and passed to `Context::decrypt_inbound_eql` (L1185-L1188).
Any `Err` is logged with `error = ?err` at `warn` and replaced by `InvalidInboundEqlPayload` (L1189-L1197).
The replaced errors include `ProtocolStateUnavailable` (`context/mod.rs` L1642), `MissingKeysetIdentifier` (`zerokms.rs` L397), every `init_cipher` failure (`zerokms.rs` L218-L229), `SteVecMissingRootEntry` (`zerokms.rs` L478), `SteVecSelectorInvalid` (`zerokms.rs` L102-L110), cipher decrypt failures (`zerokms.rs` L455) and `Plaintext::from_slice` failures (`zerokms.rs` L464).

### 4.4 Re-derivation, L1199-L1219

`verification_columns` is the queued columns, all `Some` (L1203-L1206).
`Context::encrypt(plaintexts, &verification_columns)` produces `derived` (L1207-L1210).
Any `Err` is logged and replaced by `InvalidInboundEqlPayload` (L1211-L1219).
The plaintexts recovered at L1185 are moved into this call and are not logged by this function.

### 4.5 Comparison and acceptance, L1221-L1239

`positions.into_iter().zip(derived)` (L1221) pairs each queued payload with its derived output by position.
`Context::encrypt` returns `Err` when its result length differs from the plaintext count (`context/mod.rs` L1667-L1673), so on the `Ok` path the zip covers every queued position.

A derived value that is not `Some(EqlOutput::Store(_))` returns `InvalidInboundEqlPayload` (L1222-L1229).
This arm is reached when the derived value is `None` or `EqlOutput::Query`.

`inbound_eql::sem_terms_match(&ciphertext, derived)` returning `false` returns `InvalidInboundEqlPayload` (L1230-L1237).

On a match, `encrypted[index] = Some(EqlOutput::Store(ciphertext))` (L1238).
The value written is the client-supplied ciphertext, not the derived one; the derived value is consumed by `sem_terms_match` and dropped.

Store positions are written one at a time inside the loop, so an `Err` at position k leaves positions before k already written in the caller's slice; both callers discard the slice on `Err`.

## 5. Callees

### 5.1 `inbound_eql::parse` (upstream producer of `inbound`), `inbound_eql.rs` L31-L84

`merge_inbound_eql` does not call `parse`, but its Query arm depends on `parse` entirely and its Store arm depends on `parse` for destination binding.
Paths:

1. Selector path (L36-L48): taken when `query_operand` is true, `column.eql_term` is `JsonAccessor` or `JsonPath`, and the bytes are exactly 32 lowercase hex characters (L87-L92). Returns `Query(Selector)` after `validate_query_metadata` requires a SteVec index on the column (L168-L178).
2. Not JSON, or JSON that is not an object: `Ok(None)` (L50-L55).
3. Storage-shaped, meaning keys `v` and `i` plus one of `c`, `h`, `sv` (L57-L59): deserialised as `EqlCiphertext`, then `validate_storage_metadata`; returns `Store` (L60-L65). `query_operand` is not consulted on this path.
4. Neither scalar-query-shaped nor SteVec-query-shaped: `Ok(None)` (L67-L75).
5. Query-shaped with `query_operand == false`: `Err` (L76-L78).
6. Query-shaped with `query_operand == true`: deserialised as `EqlQueryPayload`, then `validate_query_metadata`; returns `Query` (L80-L83).

What `parse` establishes for a `Store` result (`validate_storage_metadata`, L94-L125):

- `version() == EQL_SCHEMA_VERSION_V3` and `identifier() == column.identifier` (L98-L102).
- The record descriptor (`payload.ciphertext.descriptor` for scalar, `payload.key_header.descriptor` for SteVec) equals `"{table}/{column}"` (L107-L114).
- Scalar: the column has no SteVec index, and presence of `hm`, `bf`, `ob`, `op` equals presence of Unique, Match, Ore, Ope indexes (L250-L288).
- SteVec: the column has a SteVec index and `ste_vec` is non-empty (L118-L123).
- `column.eql_term` is not read anywhere in `validate_storage_metadata`.

What `parse` establishes for a `Query` result (`validate_query_metadata`, L127-L180):

- `Query` is returned only when `query_operand` is true (L36, L76).
- `Encrypted`: version and identifier equal the column's (L130-L132); for `Full`/`Partial`/`Tokenized` term presence equals index presence (L135-L143); for `JsonOrd` a SteVec index exists, `op` is present and `hm`/`bf`/`ob` are absent (L144-L154); every other `eql_term` is refused (L155).
- `SteVec`: SteVec index exists, `eql_term` is `Full`, `Partial` or `JsonValueSelector`, and `ste_vec` is non-empty (L158-L167). No `v` or `i` exists in this shape (`object.len() == 1` at L72), so no identifier is compared.
- `Selector`: SteVec index exists, `eql_term` is `JsonAccessor` or `JsonPath`, 32 lowercase hex characters (L168-L178).
- No check on a Query payload involves key material. The term values themselves (`hm`, `bf`, `ob`, `op`, selectors, SteVec terms) are accepted as supplied.

### 5.2 `Context::decrypt_inbound_eql`, `context/mod.rs` L1685-L1693

Reads the effective keyset under the `protocol_state` read lock (`context/mod.rs` L1638-L1644), releases the lock, and delegates to `EncryptionService::decrypt_inbound_eql` (trait at `proxy/mod.rs` L172-L176).
No result-length check exists here, unlike `Context::encrypt`.

### 5.3 `ZeroKms::decrypt_inbound_eql` / `decrypt_eql`, `zerokms.rs` L377-L470

Called with `SteVecAuthentication::AllEntries` (L382).
Paths:

1. No connection keyset and no default keyset: `Err(MissingKeysetIdentifier)` (L396-L398).
2. `init_cipher` fails: `Err` (L400).
3. Any ciphertext whose record `keyset_id` differs from `cipher.keyset_id()`, including a record with `keyset_id == None`: `Err(InvalidInboundEqlPayload)` (L401-L408, L485-L490). This check runs only for `AllEntries`.
4. Scalar payload: one `V3Record::Scalar` from `payload.ciphertext`, mapped to its input index (L423-L426).
5. SteVec payload: one `V3Record::SteVecEntry` per entry, built by `key_header.record_with_selector(entry.ciphertext, selector)` with the selector hex-decoded to 16 bytes (L427-L438). Only entry 0 maps to an output index (L437). An empty `ste_vec` returns `Err(SteVecMissingRootEntry)` (L476-L478).
6. `cipher.decrypt(records, &opts)` fails: `Err` (L452-L455).
7. Success: `result_positions.zip(decrypted)` writes `Plaintext::from_slice(bytes)` at mapped indexes (L461-L466). No comparison of `decrypted.len()` with `records_to_decrypt.len()` exists. A short `decrypted` leaves trailing positions `None`.

What the caller depends on:

- Every record component of every Store payload passed AEAD authentication under the cipher for the connection's effective keyset. Established by `cipher.decrypt` (black box) at L452-L455.
- Each SteVec entry is authenticated against its own selector as nonce and AAD input. Stated by the comment at L49-L51 and by `V3Record::aad_selector` forwarding to `RecordWithNonce` (L91-L96); the binding itself is inside the black box.
- The plaintext returned for a SteVec payload is the root entry's plaintext only (L437). Plaintexts of non-root entries are discarded after authentication (L459-L460).
- A position left `None` here reaches L1222 as a `None` derived value: `ZeroKms::encrypt` skips `None` plaintexts (`zerokms.rs` L260) and `Context::encrypt` returns all-`None` when every plaintext is `None` (`context/mod.rs` L1652-L1656).
- Outputs of `cipher.decrypt` are in input order. Nothing in this crate checks it.

### 5.4 `Context::encrypt`, `context/mod.rs` L1647-L1675

Paths:

1. All plaintexts `None`: returns `plaintexts.len()` `None` values without calling the service (L1652-L1656).
2. Reads the effective keyset (L1658). This is a second, independent read from the one at `context/mod.rs` L1689.
3. Service error: propagated (L1661-L1664).
4. Result length differs from plaintext count: `Err(EncryptionResultCountMismatch)` (L1667-L1673).
5. Otherwise `Ok(encrypted)`.

### 5.5 `ZeroKms::encrypt`, `zerokms.rs` L240-L361

The operation is chosen from `col.eql_term` (L262-L326):

- `Full`, `Partial`, `Tokenized`: `EqlOperation::Store` (L264-L266).
- `JsonPath`, `JsonAccessor`, `JsonOrd`, `JsonValueSelector`: `EqlOperation::Query` when the column has a SteVec index, else `EqlOperation::Store` (L269-L316).
- `JsonExtracted`: `Err(JsonExtractedIsNotAnOperand)` (L323-L325).

Consequence for the caller, combined with L1222: a Store payload is accepted only when the destination column's `eql_term` leads to `EqlOperation::Store`, which is `Full`/`Partial`/`Tokenized`, or one of the four JSON operand variants on a column with no SteVec index.
`validate_storage_metadata` does not test `eql_term`, so this gate exists only at L1222 through the derived variant.
That the black-box `encrypt_eql_v3` returns `EqlOutput::Store` for `EqlOperation::Store` and `EqlOutput::Query` for `EqlOperation::Query` is not checked in this crate.

`place_ciphertexts` returns `Err` when the black box returns a different count than was requested (`zerokms.rs` L502-L508).

The re-encryption uses `col.config` and `col.identifier` from the destination column (L328-L333) and the plaintext exactly as decrypted.
No comparison between the decrypted `Plaintext` variant and `col.config.cast_type` exists in this crate on this path.

### 5.6 `inbound_eql::sem_terms_match`, `inbound_eql.rs` L195-L221

Scalar pair (L197-L204) compares: `version`, `identifier`, `hmac_256`, `bloom_filter` as sorted sequences (L233-L248), `ore_block_u64_8_256`, `ope_cllw`.
Scalar fields not compared: the whole `ciphertext` record (`iv`, `ciphertext`, `tag`, `descriptor`, `keyset_id`, `decryption_policy`; field list from the test fixture at L319-L326).

SteVec pair (L205-L218) compares: `version`, `identifier`, `ste_vec.len()`, and per entry in positional order `selector`, `is_array`, and `term` through JSON serialisation (L223-L231).
SteVec fields not compared: `kind`, `key_header`, and each entry's `ciphertext` (field list from the fixture at L348-L365).

Mixed variants return `false` (L219).

The comparison is exact equality, so it relies on the black-box encryption producing identical `hm`, `ob`, `op`, SteVec selectors, SteVec terms and SteVec entry order for the same plaintext, column config and keyset.
Only the bloom filter is treated as order-insensitive (L236-L238).

### 5.7 Downstream consumers of the accepted value

- `project_query_operand` (L1522-L1533): in a query position, an accepted `Store` payload is converted with `into_query_operand()` (black box); a `Query` payload passes through unchanged.
- `Bind::apply_encrypted` (`bind.rs` L323-L345): `Query(Selector(s))` is written as raw text; every other value is written as its JSON serialisation.
- `Bind::apply_output` (`bind.rs` L315-L318): a position with a column and no value is rewritten to NULL.

## 6. Invariants

1. Only `InvalidInboundEqlPayload` leaves this function as an error (L1170, L1196, L1218, L1228, L1236).
2. A Store payload is written to `encrypted` only after `decrypt_inbound_eql` returned `Ok`, `Context::encrypt` returned `Ok`, the derived value is `EqlOutput::Store`, and `sem_terms_match` returned `true` (L1185-L1238).
3. A Store payload with no destination column is refused before any service call (L1169-L1171).
4. A Query payload is written with no check inside this function (L1165-L1167).
5. With no Store payload, no decrypt or encrypt call is made here (L1177-L1179).
6. A Store payload whose record keyset differs from the cipher's keyset, or that carries no keyset id, is refused (`zerokms.rs` L401-L408).
7. A position whose plaintext is missing after decrypt is refused (L1222-L1229 with `zerokms.rs` L260 and `context/mod.rs` L1652-L1656).
8. The accepted Store value is the client's bytes as deserialised, including its record ciphertext and key header; the derived ciphertext is never forwarded (L1238).

## 7. Assumptions

1. `encrypted.len()` is at least the zip length. Established by the callers through `Context::encrypt` (`context/mod.rs` L1667), not by this function (L1166, L1238).
2. `inbound.len() <= columns.len()`. Established by the callers (L646-L648; `bind.rs` L133 with L1105-L1109). The zip at L1163 does not report a mismatch.
3. Every `InboundEql::Query` reaching L1165 passed `validate_query_metadata` with `query_operand == true`. Established by `inbound_eql::parse` L36, L46, L76, L82. Nothing in this function re-checks it, and the enum is constructible directly (test at L2872).
4. The SEM term values inside a Query payload were derived from the connection's effective keyset and from the value the client intends. Nothing found. `validate_query_metadata` tests shape, version and identifier only (`inbound_eql.rs` L127-L180).
5. A SteVec Query payload belongs to the destination column. Nothing found. That shape carries no identifier (`inbound_eql.rs` L72, L158-L167).
6. Every `InboundEql::Store` reaching L1168 passed `validate_storage_metadata` for the same column passed in `columns`. Established by `inbound_eql::parse` L63 at both callers, which use the same column for `parse` and for `columns` (L648-L656 with L674; `bind.rs` L136-L153 with L1105-L1109).
7. The record descriptor compared at `inbound_eql.rs` L107-L114 is covered by the AEAD tag. Stated by the comment at `inbound_eql.rs` L104-L106; the binding is inside `cipherstash-client` (black box).
8. `cipher.decrypt` authenticates each record and returns outputs in input order. Black box; called at `zerokms.rs` L452-L455.
9. `cipher.keyset_id()` returns the keyset the cipher was initialised for. Black box; used at `zerokms.rs` L405.
10. The effective keyset is the same for the decrypt (L1187) and the re-encrypt (L1209). Nothing found in this function. Each call reads `keyset_identifier` separately (`context/mod.rs` L1658, L1689) with awaits between them.
11. `encrypt_eql_v3` is deterministic for every compared term and for SteVec entry order. Nothing found in this crate; `sem_terms_match` relies on it (`inbound_eql.rs` L198-L217).
12. `encrypt_eql_v3` returns `EqlOutput::Store` exactly for `EqlOperation::Store` inputs. Black box; relied on at L1222.
13. Authenticating a non-root SteVec entry under its selector is sufficient to tie that entry's ciphertext to the root plaintext. Nothing found. Non-root plaintexts are discarded (`zerokms.rs` L437, L459-L460) and entry ciphertexts are excluded from `sem_terms_match` (`inbound_eql.rs` L213-L217).
14. `decryption_policy` on an inbound record is verified during key retrieval. Stated only by the comment at `zerokms.rs` L410-L412, and only for tag-version 1 records.
15. The decrypted `Plaintext` variant suits the destination column's `cast_type`. Nothing found in this crate on this path (`zerokms.rs` L328-L333).
16. Nothing binds an accepted Store payload to a particular row or to a single use. The checks bind to table, column and keyset (`inbound_eql.rs` L98-L114; `zerokms.rs` L401-L408).
17. The number of SteVec entries per payload and the number of Store payloads per statement have no bound in this function; each entry becomes one decrypt record (`zerokms.rs` L430-L438).

## 8. Shared state touched

- `ConnectionProtocolState.settings` (read, twice, through `keyset_identifier`).
- `ZeroKms.cipher_cache` (read, and written on a miss, through `init_cipher`, `zerokms.rs` L162, L200-L202).
- No Prometheus counter is incremented in this function. `ENCRYPTION_ERROR_TOTAL` is incremented by the callers only for the first `Context::encrypt` (L670-L672, L1119-L1121), not for an `Err` from the merge (L674-L675, L1123-L1124).

## 9. Open questions

1. Unclear whether the effective keyset can change between L1187 and L1209; need to inspect which task runs `Context::settings_execution_succeeded` (`context/lifecycle.rs` L454) relative to the frontend task, and pg-proto's ordering guarantees.
2. Unclear how `EqlCiphertextV3` deserialises an object with `v`, `i` and `sv` but no `h`, or with both `c` and `sv`; need to inspect the serde definitions in `cipherstash-client` 0.42.2 `eql`.
3. Unclear which values `SteVecKind` admits and what consumes `kind` downstream; need to inspect `cipherstash-client` `eql::SteVecKind` and the EQL v3 SQL domain definitions. `kind` is not compared at `inbound_eql.rs` L205-L218.
4. Unclear whether `ob`, `op`, SteVec terms and SteVec entry order are deterministic across independent encryptions; need to inspect `encrypt_eql_v3` or the integration tests that send inbound Store payloads.
5. Unclear whether `encrypt_eql_v3` rejects a `Plaintext` whose variant differs from `ColumnConfig.cast_type`; need to inspect `cipherstash-client` `eql::PreparedPlaintext` and `encrypt_eql_v3`.
6. Unclear what `RecordWithNonce`/`record_with_selector` bind into the AAD and nonce; need to inspect `cipherstash-client` `zerokms::KeyHeader::record_with_selector` and `Decryptable`.
7. Unclear whether `decryption_policy` is authenticated for record tag versions other than 1; need to inspect `cipherstash-client` `zerokms` key retrieval.
8. Unclear what `EqlCiphertextV3::into_query_operand` keeps from an accepted Store payload; need to inspect `cipherstash-client` `eql`.
9. Unclear what the `?err` Debug output at L1194 and L1216 contains for `cipherstash-client` error types; need to inspect their `Debug` implementations.
10. Unclear what the upstream EQL v3 domain CHECK constraints require of a Query payload's term values; need to inspect the EQL SQL installed by the migrate command.
