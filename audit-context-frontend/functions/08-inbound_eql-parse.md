# inbound_eql::parse

- File: `packages/cipherstash-proxy/src/postgresql/inbound_eql.rs`
- Lines: L31-L84 (`parse`), with its private helpers `is_selector_hash` L87-L92, `validate_storage_metadata` L94-L125, `validate_query_metadata` L127-L180, `has_ste_vec_terms` L182-L188, `validate_scalar_terms` L250-L261, `validate_scalar_term_presence` L263-L288.
- Signature: `pub fn parse(bytes: &[u8], column: &Column, query_operand: bool) -> Result<Option<InboundEql>, EncryptError>`
- Callers: `Bind::inbound_eql` (postgresql/rewrite/bind.rs L153) and `Frontend::encrypt_literals` (postgresql/middleware/frontend.rs L653).

Paths below are relative to `packages/cipherstash-proxy/src/` unless they start with `packages/`.
"IE" abbreviates `postgresql/inbound_eql.rs`, "FE" abbreviates `postgresql/middleware/frontend.rs`, "BIND" abbreviates `postgresql/rewrite/bind.rs`, "ZK" abbreviates `proxy/zerokms/zerokms.rs`.
The format reference files named in the task (`ANALYSIS_FORMAT.md` and siblings) were not found on disk, so this document follows the layout of the sibling file `06-Frontend-encrypt_params.md`.
The source of `cipherstash_client` 0.42.2 (pinned at `Cargo.toml` L50) and of `serde_json` 1.0.151 (`Cargo.lock` L4346-L4347) was not found on disk, so every type and function from those crates is treated as a black box and its assumed behaviour is listed in section 6.

## 1. Purpose

`parse` is a classifier with three outcomes for one client-supplied value destined for one encrypted column:

- `Ok(None)`: the value is not an EQL payload. Both callers then treat it as plaintext and encrypt it (`skip` is false for that position, FE L661-L662 and FE L1096-L1101).
- `Ok(Some(InboundEql::Store(_)))` or `Ok(Some(InboundEql::Query(_)))`: the value is client-generated EQL. Both callers skip plaintext conversion for that position (FE L661, FE L1096) and hand the payload to `merge_inbound_eql` (FE L674, FE L1156-L1241).
- `Err(EncryptError::InvalidInboundEqlPayload)`: the value has the key shape of an EQL payload and failed a check. It is the only error variant `parse` returns (IE L44, L62, L77, L81, and every `Err` in the helpers).

The function is pure: it reads its three arguments, allocates, and returns.
It touches no connection state, no schema store, no keyset, and performs no I/O.
It performs no cryptographic check; the doc comment at IE L19-L20 states that a `Store` payload "must be authenticated and have its SEM terms independently verified before it can be used", and that work is in `Frontend::merge_inbound_eql` (FE L1185-L1238).

## 2. Inputs and who controls them

| Input | Origin | Controlled by |
|---|---|---|
| `bytes` (Bind path) | `param.bytes`, or `param.json_bytes()` when the param is binary format and its first byte is `0x01` (BIND L148-L152; `json_bytes` drops byte 0 for binary params, BIND L511-L517) | Authenticated client: content, length, format code |
| `bytes` (literal path) | `literal.clone().into_string()` then `.as_bytes()` (FE L650, L654) | Authenticated client, through SQL text as parsed by sqltk |
| `column` | `output.column` of the stored `Statement` (BIND L136), or `literal_columns` (FE L648-L650) | Parse-time schema and encrypt-config snapshot; `eql_term` is the mapper's inferred term for that position |
| `query_operand` (Bind path) | `output.query_operand` (BIND L153). Set by the mapper as `is_selector \|\| query_operands.contains(&output)` (packages/eql-mapper/src/type_checked_statement.rs L238-L248) and copied at context/statement.rs L158. It is the constant `false` for a statement that was never transformed (FE L939-L947). | EQL Mapper output over client SQL |
| `query_operand` (literal path) | `literal_is_query_operand`: true when `column.eql_term` is `JsonAccessor` or `JsonPath`, or when the mapper recorded the literal in `query_operands` (FE L1538-L1547) | EQL Mapper output over client SQL |

Preconditions the Bind caller applies before calling: the output has a column (BIND L136-L138), the source is `OutputParamSource::Input` (L139-L141), the input index exists (L142-L144), the param is not NULL (L145-L147).
The literal caller requires `Some(column)` and a literal that `into_string()` accepts (FE L650-L652).
Neither caller bounds the length of `bytes` or consults the declared type OID before calling.

## 3. Block-by-block

### 3.1 Bare selector-hash branch (IE L36-L48)

Entered only when all three hold: `query_operand` is true (L36), `column.eql_term` is `JsonAccessor` or `JsonPath` (L37-L40), and `is_selector_hash(bytes)` (L41).
`is_selector_hash` (L87-L92) is true exactly when `bytes.len() == 32` (constant at L14) and every byte is an ASCII digit or `b'a'..=b'f'`.
Uppercase hex does not match (test at IE L644-L646).

Inside the branch:

- L43-L44 converts to `String`. Every byte accepted at L91 is ASCII, so `String::from_utf8` cannot fail here and the `map_err` arm is unreachable.
- L45 builds `EqlQueryPayload::Selector(selector)`. The string is the client's bytes unchanged.
- L46 calls `validate_query_metadata`, `Selector` arm (L168-L178). It fails unless `has_ste_vec_terms(column)`, `column.eql_term` is `JsonAccessor | JsonPath`, and `is_selector_hash(selector)`. The second and third conditions repeat what L37-L41 already established, so the only new condition is `has_ste_vec_terms`.
- L47 returns `Ok(Some(InboundEql::Query(Selector)))`.

Consequences of this ordering:

- When the three entry conditions hold and the column has no `SteVec` index, the result is `Err`, not `Ok(None)`. The value is not re-tried as JSON.
- The JSON branch is never reached for a 32-character lowercase-hex value in a selector query position. A plaintext selector that is 32 lowercase hex characters is classified as an already-tokenized selector. The module comment at IE L9-L13 states this resolution is intentional.
- No identifier, keyset, or version is associated with a `Selector`; the type carries only a string (L45, L168). Nothing in `parse` ties the selector to `column.identifier`.
- Downstream, `InboundEql::Query` is written into the output slot without further checks (FE L1165-L1167), and a `Selector` is emitted as raw text: `param.rewrite_text(s.clone().into_bytes())` on Bind (BIND L333-L335) and `Value::SingleQuotedString(s.clone())` for literals (FE L732-L734).

When any entry condition is false, control falls through to 3.2 with the same `bytes`.
In particular, with `query_operand == false` a 32-hex value on a `JsonAccessor` column is passed to the JSON parser (L50), fails to parse as an object, and returns `Ok(None)`.

### 3.2 JSON gate (IE L50-L55)

- L50-L52: `serde_json::from_slice::<Value>(bytes)`. Any parse failure returns `Ok(None)`.
- L53-L55: a parsed value that is not a JSON object returns `Ok(None)`.

So every non-JSON value, every JSON scalar, and every JSON array is classified as plaintext without error.
The whole of `bytes` is parsed into an owned `serde_json::Value` tree for every non-NULL value bound to an encrypted column, before any shape test.
`parse` places no length bound on `bytes` and none is applied by either caller (BIND L148-L153, FE L650-L654).

### 3.3 Storage-shaped branch (IE L57-L65)

`storage_shaped` (L57-L59) is a key-presence test only: keys `v` and `i` are present, and at least one of `c`, `h`, `sv` is present.
The values under those keys are not inspected at this point.

When true:

- L61-L62: `serde_json::from_value::<EqlCiphertext>(value)`; failure maps to `Err(InvalidInboundEqlPayload)`. There is no fall-through to plaintext. Test: IE L383-L388.
- L63: `validate_storage_metadata(&ciphertext, column)?`.
- L64: `Ok(Some(InboundEql::Store(ciphertext)))`.

This branch does not read `query_operand`.
A storage-shaped object is returned as `Store` in a query position as well as in a storage position.
The conversion of a `Store` in a query position into a query payload happens later in `project_query_operand` (FE L1522-L1533), after `merge_inbound_eql` has run.

Ordinary JSON objects that contain `v`, `i`, and one of `c`/`h`/`sv` as application keys are routed here and produce `Err` when they do not deserialize as `EqlCiphertext`.
The doc comment at IE L27-L30 names this behaviour ("malformed advertised payloads fail closed").

#### validate_storage_metadata (IE L94-L125)

Checks, in order:

1. L98-L102: `ciphertext.version() == EQL_SCHEMA_VERSION_V3` and `ciphertext.identifier() == &column.identifier`. Both are fields of the client-supplied envelope.
2. L107-L114: the record descriptor equals `format!("{}/{}", column.identifier.table, column.identifier.column)`. For `Encrypted` the descriptor read is `payload.ciphertext.descriptor` (L109); for `SteVec` it is `payload.key_header.descriptor` (L110).
3. L116-L124, per variant:
   - `Encrypted`: `validate_scalar_terms` (L250-L261), which forwards the presence of `hmac_256`, `bloom_filter`, `ore_block_u64_8_256`, `ope_cllw` to `validate_scalar_term_presence`.
   - `SteVec`: `has_ste_vec_terms(column)` and `!payload.ste_vec.is_empty()` (L119).

What this function does not read:

- `column.eql_term`. A storage payload is accepted by `parse` for any term variant of the destination position as long as the index configuration matches.
- `keyset_id` and `decryption_policy` on the record or key header. The keyset comparison is in `ZeroKms::decrypt_eql` (ZK L401-L408) and applies only on the `AllEntries` path used by `decrypt_inbound_eql` (ZK L377-L384). `decryption_policy` is not compared anywhere in this crate (comment at ZK L410-L412).
- `kind` on a `SteVec` payload (the field exists: IE L350).
- The content of any SEM term. Only presence is checked. Content is compared in `sem_terms_match` (IE L195-L221) from `merge_inbound_eql` (FE L1230).
- Any per-entry field of `ste_vec` (`selector`, `ciphertext`, `is_array`, `term`); only non-emptiness of the vector is checked (L119).
- For a `SteVec` payload, the other index types configured on the column. `has_ste_vec_terms` (L182-L188) is true when any index is `SteVec`.

The comment at L104-L106 states the descriptor "is covered by the encrypted record's AEAD tag".
Nothing in `parse` verifies a tag.
At the point `parse` returns `Store`, the descriptor comparison is a comparison of two client-supplied strings (`i` and the descriptor) against the destination column.
The binding the comment describes exists only after `decrypt_inbound_eql` succeeds (FE L1185-L1197).

The expected descriptor is built by joining table and column with `/` (L107).
The identifier equality at L99 is a separate, field-wise comparison and runs first.
`Identifier` is constructed from two strings (IE L302) and exposes `table` and `column` (L107); no schema component is visible in this file.

#### validate_scalar_term_presence (IE L263-L288)

Builds four booleans from `column.config.indexes` (L274-L282): `Unique` sets hmac, `Match` sets bloom, `Ore` sets ore, `Ope` sets ope.
Any `SteVec` index on the column returns `Err` immediately (L280), so a scalar payload is never accepted for a column that has a SteVec index (test IE L473-L480).
L284 requires each payload-presence flag to equal the corresponding configured flag.
The comparison is exact in both directions: a configured term that is absent fails, and a present term that is not configured fails.
A column with zero indexes accepts only a payload with all four terms absent.

### 3.4 Query-shaped branch (IE L67-L83)

Reached only when the object is not storage-shaped, which means: `v` or `i` is missing, or none of `c`, `h`, `sv` is present.

- `scalar_query_shaped` (L67-L71): keys `v` and `i` present and at least one of `hm`, `bf`, `ob`, `op` present. Because the storage test already failed, such an object has none of `c`, `h`, `sv`.
- `ste_vec_query_shaped` (L72): the object has exactly one key and that key is `sv`.
- L73-L75: neither shape returns `Ok(None)`. Examples that land here: `{"v":..,"i":..}` with no term key, `{"sv":[..],"x":1}`, `{"c":"..."}` (test IE L376-L380), `{"hm":".."}` without `v`/`i`.
- L76-L78: a query-shaped object with `query_operand == false` returns `Err`. Tests IE L503-L520 and L609-L616.
- L80-L81: `serde_json::from_value::<EqlQueryPayload>(value)`; failure maps to `Err`.
- L82: `validate_query_metadata(&query, column)?`.
- L83: `Ok(Some(InboundEql::Query(query)))`.

An object with `v`, `i`, and `sv` but no `c`/`h` is storage-shaped (L59 counts `sv`), not query-shaped, so the only SteVec query form reachable at L72 is the single-key object.

#### validate_query_metadata (IE L127-L180)

`Encrypted` arm (L129-L157):

- L130: `payload.version == EQL_SCHEMA_VERSION_V3` and `payload.identifier == column.identifier`.
- L134-L156 dispatches on `column.eql_term`:
  - `Full | Partial | Tokenized` (L135-L143): `validate_scalar_term_presence` with the same exact-match rule as storage. A query operand must carry every term the column configures and no other. A column with any SteVec index fails (L280).
  - `JsonOrd` (L144-L154): requires `has_ste_vec_terms(column)`, `hm`, `bf`, `ob` all absent, and `op` present.
  - Every other variant (`JsonAccessor`, `JsonPath`, `JsonValueSelector`, `JsonExtracted`; enum at packages/eql-mapper/src/inference/unifier/types.rs L254-L271): `Err` (L155).

`SteVec` arm (L158-L167): requires `has_ste_vec_terms(column)`, `column.eql_term` in `Full | Partial | JsonValueSelector`, and a non-empty `ste_vec`.
No version and no identifier are compared; the shape test at L72 admits only the single key `sv`, so the JSON form that reaches this arm through L72 carries neither.
No per-entry field is checked. The test at IE L599-L606 passes an entry whose selector is the string `"application-generated selector"`, which is not in the 32-hex format, and it is accepted.

`Selector` arm (L168-L178): described in 3.1.
Whether `serde_json::from_value::<EqlQueryPayload>` at L80 can yield `Selector` from a JSON object is not established from this file (see open questions); the shape tests at L67-L72 admit only objects.

What `validate_query_metadata` does not check, for any arm:

- The content, length, or encoding of any term. The tests accept `hm` as an arbitrary sentence (IE L493-L499) and `op` as an arbitrary sentence (IE L546-L556).
- Any relation between the terms and a keyset. A query payload carries no ciphertext, and the doc comment at IE L22-L23 states it "has no source ciphertext with which to authenticate its metadata".

Downstream, a `Query` payload is placed in the output slot with no further validation in this crate (FE L1165-L1167); `merge_inbound_eql`'s doc comment says it is "accepted only after role-aware structural validation in `inbound_eql::parse`" (FE L1153-L1155).
`parse` is therefore the only check in Proxy between a client-supplied query payload and the rewritten statement or Bind sent upstream.

## 4. Decision table

| Condition (first match wins) | Result | Lines |
|---|---|---|
| `query_operand` and `eql_term` in {JsonAccessor, JsonPath} and bytes are 32 lowercase hex, column has SteVec index | `Query(Selector(bytes))` | L36-L47, L168-L177 |
| same, column has no SteVec index | `Err` | L46, L173-L175 |
| bytes are not JSON | `None` | L50-L52 |
| JSON, not an object | `None` | L53-L55 |
| object with `v`, `i`, and one of `c`/`h`/`sv`; deserializes and passes storage validation | `Store` | L57-L64 |
| same shape; deserialization or validation fails | `Err` | L62-L63 |
| object with `v`, `i`, one of `hm`/`bf`/`ob`/`op`, or object `{sv}` only; `query_operand` false | `Err` | L67-L78 |
| same shape; `query_operand` true; deserializes and passes query validation | `Query` | L80-L83 |
| same shape; `query_operand` true; deserialization or validation fails | `Err` | L81-L82 |
| any other object | `None` | L73-L75 |

## 5. Invariants

- I1. The only error `parse` returns is `EncryptError::InvalidInboundEqlPayload` (IE L44, L62, L77, L81, L101, L113, L120, L131, L151, L155, L164, L175, L280, L285). Its message carries no payload detail (error.rs L310-L317) and it maps to one client diagnostic (postgresql/error_handler.rs L61-L63).
- I2. `InboundEql::Query` is returned only when `query_operand` is true (L36 guards the return at L47; L76-L78 guards the return at L83).
- I3. `InboundEql::Store` is returned only after `validate_storage_metadata` returned `Ok` (L63-L64), so its envelope version is `EQL_SCHEMA_VERSION_V3`, its `i` equals `column.identifier`, and its descriptor equals `"{table}/{column}"` (L98-L114).
- I4. A returned `Store(Encrypted)` has SEM-term presence exactly equal to the column's `Unique`/`Match`/`Ore`/`Ope` configuration and the column has no `SteVec` index (L274-L287).
- I5. A returned `Store(SteVec)` has a non-empty `ste_vec` and the column has at least one `SteVec` index (L119).
- I6. A returned `Query(Encrypted)` has version V3 and `identifier == column.identifier` (L130), and `column.eql_term` is `Full`, `Partial`, `Tokenized`, or `JsonOrd` (L134-L155).
- I7. A returned `Query(Selector(s))` has `s` matching `^[0-9a-f]{32}$`, a column with a `SteVec` index, and `eql_term` in `JsonAccessor | JsonPath` (L168-L177).
- I8. A returned `Query(SteVec)` has a non-empty `ste_vec`, a column with a `SteVec` index, and `eql_term` in `Full | Partial | JsonValueSelector` (L159-L165).
- I9. Shape classification depends only on top-level key presence, never on the values under those keys (L57-L59, L67-L72).
- I10. `parse` reads and writes no state outside its arguments (L31-L84; no `self`, no statics).

## 6. Assumptions

| # | Assumption | Established by |
|---|---|---|
| A1 | A returned `Store` payload is authenticated and its SEM terms are re-derived and compared before it is used. | Not in `parse`. `Frontend::merge_inbound_eql` FE L1185-L1238 (`decrypt_inbound_eql`, re-`encrypt`, `sem_terms_match`). Both callers of `parse` pass the result to it (FE L674, FE L1095-L1096 then the merge call inside `encrypt_params`). A third caller of `parse` would have to do the same; nothing in the type of `InboundEql::Store` forces it. |
| A2 | The descriptor compared at L112 is bound to the ciphertext by an AEAD tag. | Comment at IE L104-L106 only. Verification is external (`cipher.decrypt`, ZK L452-L455, in `cipherstash_client`, source not available). |
| A3 | The keyset named in the payload is the connection's effective keyset. | Not in `parse`. ZK L401-L408 on the `decrypt_inbound_eql` path. |
| A4 | A returned `Query` payload needs no authentication; its terms are whatever the client sent. | Stated at IE L22-L23 and FE L1153-L1155. No check of term content, length, or encoding found in this crate; any such check is either in the external deserializer (source not available) or in PostgreSQL's `eql_v3.query_*` domains (referenced in the comment at FE L1514-L1515, not in this repository's Rust code). |
| A5 | `query_operand` correctly reflects whether the value sits in a predicate position rather than a stored position. | EQL Mapper (packages/eql-mapper/src/type_checked_statement.rs L238-L248; `QueryOperands`, packages/eql-mapper/src/query_operands.rs L49-L56). For an untransformed statement it is constant `false` (FE L945). `parse` cannot check it. |
| A6 | `column` describes the column this value will reach, including `identifier`, `config.indexes`, and `eql_term`. | Parse-time snapshot held in the stored `Statement` (BIND L136) or `literal_columns` (FE L648). `parse` does not re-derive it. |
| A7 | In a selector query position a 32-lowercase-hex value is an application-generated token and not a plaintext selector. | Nothing establishes it; IE L9-L13 documents the ambiguity as resolved by convention. |
| A8 | A `Selector` token belongs to the destination column and the effective keyset. | nothing found. The `Selector` arm compares no identifier or keyset (L168-L178). |
| A9 | A `Query(SteVec)` payload belongs to the destination column. | nothing found. The `SteVec` arm compares no identifier or version (L158-L167). |
| A10 | Per-entry SteVec fields (`selector` format, `term`) are well-formed. | For `Store`: `decode_ste_vec_selector` and per-entry decrypt on the inbound path (ZK L428-L438), then `sem_terms_match` (IE L205-L217). For `Query`: nothing found in this crate. |
| A11 | `serde_json::from_slice::<Value>` bounds recursion depth and does not panic on client bytes. | External (`serde_json`), source not available. Feature flags in effect after workspace unification were not inspected. |
| A12 | `serde_json::from_value::<EqlCiphertext>` and `::<EqlQueryPayload>` select the variant the key shape implies, and their handling of unknown keys, duplicate keys, and keys from the other shape is as the validators expect. | External (`cipherstash_client::eql`), source not available. `parse` relies on the deserializer for every field-level type check. |
| A13 | `EqlCiphertext::version()` and `identifier()` return the client-supplied `v` and `i` of whichever variant was deserialized. | External (`cipherstash_client::eql`), source not available. |
| A14 | The bytes handed in are the JSON text of the value. | Bind caller strips one leading `0x01` when the param is binary (BIND L148-L152) without consulting the declared type; literal caller passes the SQL string body (FE L650-L654). |
| A15 | Input size is bounded by something upstream of `parse`. | nothing found in `parse` or its two call sites. |

## 7. Calls

| Callee | Kind | What `parse` depends on it for |
|---|---|---|
| `is_selector_hash` (IE L87-L92) | internal | Length 32 and lowercase-hex alphabet; also makes `String::from_utf8` at L43 infallible. |
| `validate_query_metadata` (IE L127-L180) | internal | All acceptance rules for `Query`: version and identifier for `Encrypted` only, term presence, `eql_term` compatibility, SteVec configuration, non-empty `ste_vec`. Returns `Ok` on exactly the paths listed in I6-I8. |
| `validate_storage_metadata` (IE L94-L125) | internal | Version, identifier, descriptor, term presence or SteVec non-emptiness. Does not read `eql_term`, keyset, policy, `kind`, or term content. |
| `validate_scalar_term_presence` (IE L263-L288) via the two validators | internal | Exact presence match against `column.config.indexes`; rejects any column with a `SteVec` index. |
| `has_ste_vec_terms` (IE L182-L188) | internal | True when any configured index is `SteVec`. |
| `serde_json::from_slice::<Value>` (L50) | external-black-box | Sent: raw client bytes. Assumed: returns `Err` for non-JSON, no panic, bounded recursion. Not excluded: large allocation proportional to input; duplicate-key resolution rule unknown. |
| `Value::as_object`, `Map::contains_key`, `Map::len` (L53-L72) | external-black-box | Top-level key presence and count. |
| `serde_json::from_value::<EqlCiphertext>` (L61-L62) | external-black-box | Sent: client JSON object. Assumed: all field typing and variant selection. Not excluded: acceptance of unknown or extra keys; acceptance of a variant the key shape did not imply. |
| `serde_json::from_value::<EqlQueryPayload>` (L80-L81) | external-black-box | Same as above for the query type, including which of `Encrypted`, `SteVec`, `Selector` an object can deserialize to. |
| `EqlCiphertext::version`, `EqlCiphertext::identifier` (L98-L99) | external-black-box | Return envelope `v` and `i`. |
| `String::from_utf8` (L43) | std | Cannot fail after L41. |

## 8. Callers

- `Bind::inbound_eql` (BIND L129-L156), called from `Frontend::encrypt_params` (FE L1095). One call per output param that has a column, an `Input` source, a bound index, and a non-NULL value.
- `Frontend::encrypt_literals` (FE L646-L660). One call per literal that has a column and a string form.

Both callers convert the error with `Error::from` and return on the first error (BIND L153-L155, FE L658-L660).
Both derive `skip` from `Option::is_some` on the result (FE L661, FE L1096), so `parse`'s `Some`/`None` decision selects between "pass the client's payload through" and "encrypt as plaintext".

## 9. Shared state

None read or written by `parse` or its helpers.

## 10. Open questions

- Unclear whether `EqlCiphertextV3`, `EncryptedPayloadV3`, `SteVecPayloadV3`, and `EqlQueryPayloadV3` reject unknown JSON keys; need to inspect `cipherstash_client::eql` 0.42.2 (serde attributes on those types).
- Unclear how `EqlCiphertextV3` and `EqlQueryPayloadV3` select a variant during deserialization (tagged, untagged, or custom) and whether a storage-shaped object containing both `c` and `sv`, or `h` without `sv`, can deserialize; need to inspect `cipherstash_client::eql`.
- Unclear whether a JSON object can deserialize to `EqlQueryPayloadV3::Selector` at IE L80; need to inspect `cipherstash_client::eql`.
- Unclear which wire key `h` denotes and which struct field it maps to; need to inspect `SteVecPayloadV3` in `cipherstash_client::eql`.
- Unclear whether `version()` / `identifier()` read the same fields for both variants; need to inspect `EqlCiphertextV3` in `cipherstash_client::eql`.
- Unclear whether term fields (`hm`, `bf`, `ob`, `op`, SteVec entry `s` and term) have any format or length check at deserialization; need to inspect `cipherstash_client::eql`. The tests at IE L493-L499, L546-L556, L599-L606 show arbitrary strings are accepted for `hm`, `op`, and a query entry selector.
- Unclear whether `Identifier` has fields beyond `table` and `column` and how its `PartialEq` treats case and schema; need to inspect `cipherstash_client::eql::Identifier`.
- Unclear how `serde_json` 1.0.151 resolves duplicate top-level keys into `Value::Object` and what recursion limit and feature flags (`arbitrary_precision`, `preserve_order`, `unbounded_depth`) are in effect for this workspace; need to inspect `Cargo.lock` feature resolution and `serde_json` source.
- Unclear what PostgreSQL-side checks the `eql_v3.query_*` domains apply to a client-supplied `Query` payload or selector token; need to inspect the EQL v3 SQL definitions (not in `packages/cipherstash-proxy/src`).
- Unclear whether any limit on Bind parameter size or SQL literal size applies before `parse` runs; need to inspect pg-proto framing limits and `Bind::try_from(PgBind)` (BIND L348 onward).
- Unclear how `sqltk`'s `Value::into_string` (FE L650) renders escaped string literals and which literal kinds it returns `None` for; need to inspect `sqltk` / `sqlparser` `Value::into_string`.
- Unclear what `merge_inbound_eql` yields for a `Store` payload accepted at a position whose `eql_term` is not `Full`/`Partial` (for example `JsonAccessor`, `JsonOrd`, `Tokenized`), since `validate_storage_metadata` does not read `eql_term`; need to inspect `Context::encrypt` / `ZeroKms::encrypt` output variant per term (ZK encrypt path) against the `EqlOutput::Store` requirement at FE L1222.
