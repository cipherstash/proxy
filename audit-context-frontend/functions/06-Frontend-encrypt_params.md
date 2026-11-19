# Frontend::encrypt_params

- File: `packages/cipherstash-proxy/src/postgresql/middleware/frontend.rs`
- Lines: L1089-L1150
- Signature: `async fn encrypt_params(&mut self, session_id: Option<SessionId>, bind: &Bind, statement: &Statement) -> Result<Vec<Option<EqlOutput>>, Error>`
- Sole caller: `Frontend::bind_handler` at frontend.rs L1046, guarded by `statement.has_params()` (L1045).

Paths below are relative to `packages/cipherstash-proxy/src/` unless they start with `packages/`.
The format reference files named in the task (`ANALYSIS_FORMAT.md` and siblings) were not found on disk, so this document uses its own section layout.

## 1. Purpose

Produces, for one Bind message, a vector of `Option<EqlOutput>` that is positional over `statement.output_params` (the params PostgreSQL receives after the rewrite), not over the params the client bound.
`Some` means "replace this output param with this EQL payload"; `None` means "nothing was produced for this position".
The function does not mutate `bind` (it takes `&Bind`, L1092).
The substitution into the wire message is done by the caller through `bind.rewrite(&statement.output_params, encrypted)` at L1047.

## 2. Inputs and who controls them

| Input | Origin | Controlled by |
|---|---|---|
| `bind.param_values`, `bind.param_format_codes` | `Bind::try_from(PgBind)` (postgresql/rewrite/bind.rs L348-L395), called at frontend.rs L279 | Authenticated client: count, bytes, NULL-ness, and text/binary format of every param |
| `statement.output_params` | Built at Parse: positional default at frontend.rs L939-L947, replaced from the mapper `ParamPlan` at L861-L865 when a transform ran | EQL Mapper output over client SQL plus the schema snapshot at Parse time |
| `statement.postgres_param_types` | Client Parse `parameter_types`, captured at frontend.rs L829-L830, stored at L949-L955 | Authenticated client |
| `session_id` | `knowledge.metrics` from `Context::begin_bind` (context/lifecycle.rs L240-L244, L254-L258) | Proxy; `None` when the prepared statement has no metrics scope (context/mod.rs L1126-L1128) |
| Effective keyset | Read inside `Context::encrypt` and `Context::decrypt_inbound_eql` (context/mod.rs L1658, L1689) | Client through `SET CIPHERSTASH.KEYSET_*` once PostgreSQL confirms it |
| `Column` inside each `OutputParam` | `ColumnMapper::get_column` (postgresql/column_mapper.rs L161-L202) at Parse | Column encrypt config snapshot at Parse time |

The `Statement` is the one stored for the prepared-statement name at Parse and looked up in `begin_bind` (context/lifecycle.rs L249).
Nothing in `encrypt_params` or `bind_handler` re-derives it at Bind.

## 3. Block-by-block

### 3.1 Inbound EQL detection (L1095-L1096)

`bind.inbound_eql(&statement.output_params)` (rewrite/bind.rs L129-L156) returns one entry per output param.
It returns `None` for a position on each of these paths:

- the output param has no column (bind.rs L136-L138);
- the source is not `OutputParamSource::Input` (L139-L141), so fused `JsonValueSelector` and `JsonAccessorPath` outputs are never inspected for inbound EQL;
- the referenced input index is not present in `param_values` (L142-L144);
- the param is NULL (L145-L147);
- `inbound_eql::parse` returns `Ok(None)`.

The bytes handed to `parse` are `param.json_bytes()` (bytes with the first byte removed) when the param is binary format and its first byte is `0x01`; otherwise the raw bytes (bind.rs L148-L152).
The choice does not consult the param's declared type OID.

`inbound_eql::parse` (postgresql/inbound_eql.rs L31-L84) classifies by shape:

- L36-L48: when `query_operand` is true, the column's term is `JsonAccessor` or `JsonPath`, and the bytes are exactly 32 characters of `[0-9a-f]` (L87-L92), the value is taken as an already-tokenized selector and returned as `InboundEql::Query(Selector)` after `validate_query_metadata`. The module comment at L11-L13 states that a plaintext selector that happens to match this format is treated as a tokenized one.
- L50-L55: bytes that are not a JSON object return `Ok(None)` (treated as plaintext downstream).
- L57-L65: an object with keys `v` and `i` and any of `c`, `h`, `sv` is "storage shaped". It must deserialize as `EqlCiphertext` and pass `validate_storage_metadata`, or the call fails with `InvalidInboundEqlPayload`. There is no fall-through to plaintext for a storage-shaped object that fails to deserialize.
- L67-L78: an object with `v`, `i` and any of `hm`, `bf`, `ob`, `op`, or an object whose only key is `sv`, is "query shaped". When `query_operand` is false this is an error (L76-L78). Otherwise it must deserialize as `EqlQueryPayload` and pass `validate_query_metadata` (L80-L83).
- Every other object returns `Ok(None)` (L73-L75).

`validate_storage_metadata` (inbound_eql.rs L94-L125) requires version `EQL_SCHEMA_VERSION_V3`, `identifier == column.identifier`, the record descriptor equal to `"{table}/{column}"`, and term presence matching the column's configured SEM terms (`validate_scalar_term_presence`, L263-L288) or, for SteVec, a SteVec-configured column and a non-empty `ste_vec`.
It does not consult `column.eql_term`.

`validate_query_metadata` (inbound_eql.rs L127-L180) checks version and identifier for the `Encrypted` variant (L130), term presence against the column config, and that the payload variant is compatible with `column.eql_term`.
For the `SteVec` and `Selector` variants no identifier is compared (L158-L178); the checks are term shape, SteVec configuration on the column, non-empty `ste_vec`, or selector format.

`skip` (L1096) is `true` exactly where `inbound` is `Some`.

### 3.2 Plaintext conversion (L1097-L1101)

`bind.to_plaintext_skipping(output_params, postgres_param_types, &skip)` (rewrite/bind.rs L72-L125) maps over `output_params`, so its result length equals `output_params.len()` on success.
Per position it yields `None` when:

- `skip[output_index]` is true (L82-L84);
- the output has no column (L85-L88);
- the source is `Input(idx)` and `param_values.get(idx)` is `None` (L100-L102) — a param the statement expects but the client did not bind yields `Ok(None)` here, not an error;
- the param is NULL (`bind_param_from_sql`, postgresql/data/from_sql.rs L26-L28);
- for `JsonValueSelector`: any path step is unbound or NULL (`resolve_selector_path`, bind.rs L242-L245; L175-L177), the value input is unbound (L179-L181), or the value is NULL (L183-L185, from_sql.rs L245-L247);
- for `JsonAccessorPath`: any path step is unbound or NULL (bind.rs L214-L216).

The declared type used for decoding is `get_param_type(input, param_types, col)` (bind.rs L90-L91, L401-L406): the client's Parse OID at the primary input index when it resolves through `Type::from_oid`, else `col.postgres_type`.
An OID of 0 or an OID unknown to `postgres_types` falls back to the column type (L404-L405).

Conversion for `Input` sources is `bind_param_from_sql` (from_sql.rs L18-L36): text format goes to `text_from_sql` (L336-L416), binary to `binary_from_sql` (L421-L521).
Any error from it is replaced by `MappingError::InvalidParameter(col)` (bind.rs L112-L114).
`text_from_sql` contains `unimplemented!("Timestamp")` for `(Full | Partial, ColumnType::Timestamp)` (from_sql.rs L381-L383) and `binary_from_sql` contains `unimplemented!("TIMESTAMPTZ")` for `(_, ColumnType::Timestamp, TIMESTAMPTZ)` (L506); both are panics, not `Err`.
Binary `BigUInt` conversion casts signed integers with `as u64` (from_sql.rs L464-L472).
A binary param declared `TEXT`/`VARCHAR` for a non-text column is re-parsed through `text_from_sql` with the term forced to `EqlTermVariant::Full` (L513-L514).

Path steps of fused selectors are read with `param.to_string()` (bind.rs L243), which is `String::from_utf8_lossy` over the raw bytes regardless of format code (bind.rs L532-L537).

### 3.3 Column vector (L1105-L1109)

`output_param_columns` clones `output.column` for each output param.
It has the same length and order as `output_params`, and therefore as `plaintexts` and `inbound`.

### 3.4 Debug log (L1111)

`plaintexts` is written with `?` formatting to the `MAPPER` target at debug level.
The caller also logs the whole `bind` at debug level on the `PROTOCOL` target (L1038).

### 3.5 Encrypt (L1113-L1121)

`self.context.encrypt(plaintexts, &output_param_columns)` (context/mod.rs L1647-L1675):

- L1652-L1656: when every plaintext is `None`, returns a vector of `None` of the same length without reading the keyset and without calling the encryption service.
- L1658: reads the effective keyset from `ConnectionSettings` at this moment (L1638-L1644).
- L1661-L1664: delegates to `EncryptionService::encrypt`.
- L1667-L1673: rejects a result whose length differs from `plaintexts.len()`.

`ZeroKms::encrypt` (proxy/zerokms/zerokms.rs L240-L361):

- L249-L251: fails when neither a default nor a connection keyset exists.
- L253: `init_cipher` resolves a `ScopedCipher` from the process-wide cache keyed by the keyset identifier string or `"default"` (L147-L152, L162-L166).
- L259-L337: zips `plaintexts` with `columns` and prepares only positions where both are `Some`. The EQL operation is chosen from `col.eql_term`: `Store` for `Full | Partial | Tokenized` (L264-L266); a `Query` op for `JsonPath`, `JsonAccessor`, `JsonOrd`, `JsonValueSelector` when the column has a SteVec SEM term, and `Store` as the fallback when it has none (L269-L316, each `.unwrap_or(EqlOperation::Store)`); an error for `JsonExtracted` (L323-L325).
- L345-L347: returns all `None` when nothing was prepared.
- L354-L356: `encrypt_eql_v3` (cipherstash_client, source not read).
- L360, L497-L514: `place_ciphertexts` rejects a ciphertext count different from the prepared count and writes each result back at its original index.

Any error increments `ENCRYPTION_ERROR_TOTAL` (L1119-L1121) and returns.

### 3.6 Merge inbound EQL (L1123-L1124)

`merge_inbound_eql` (frontend.rs L1156-L1241) walks `inbound` zipped with `columns`:

- `InboundEql::Query` is written directly into `encrypted[index]` (L1165-L1167). No ZeroKMS call is made for it; the only checks it has passed are those in `inbound_eql::parse` (doc comment L1153-L1155).
- `InboundEql::Store` is collected with its column (L1168-L1173).
- If no `Store` entries exist, returns (L1177-L1179).

For `Store` entries:

1. `context.decrypt_inbound_eql` (context/mod.rs L1685-L1693) reads the effective keyset again (L1689) and calls `ZeroKms::decrypt_eql` with `SteVecAuthentication::AllEntries` (zerokms.rs L377-L384). That path rejects any ciphertext whose embedded keyset id differs from the cipher's keyset id (L401-L408) and decrypts every SteVec entry (L427-L439, L472-L483). Any error maps to `InvalidInboundEqlPayload` (frontend.rs L1189-L1197).
2. `context.encrypt(plaintexts, &verification_columns)` re-encrypts the decrypted plaintexts for the destination columns (L1207-L1219). This is a third independent read of the effective keyset (context/mod.rs L1658).
3. Each derived value must be `Some(EqlOutput::Store(_))` (L1222-L1229) and `inbound_eql::sem_terms_match` must hold (L1230-L1237; inbound_eql.rs L195-L221). On success the *inbound* ciphertext, not the derived one, is written to `encrypted[index]` (L1238).

Step 3's `Store` requirement rejects a storage-shaped payload bound to a param whose column term makes `ZeroKms::encrypt` produce a `Query` output (JSON selector/ordering terms on a SteVec column).
`sem_terms_match` compares version, identifier, hmac, bloom filter (order-insensitive), ORE, OPE for scalars, and per-entry selector, `is_array`, and term for SteVec (inbound_eql.rs L197-L217).

Indexing `encrypted[index]` at L1166 and L1238 does not go out of bounds because `index < min(inbound.len(), columns.len()) == output_params.len()` and `encrypted.len() == output_params.len()` (context/mod.rs L1652-L1656, L1667-L1673).

### 3.7 Query-operand projection (L1126-L1128)

For each output param, `project_query_operand(output.query_operand, encrypted)` (frontend.rs L1522-L1533):

- does nothing when `query_operand` is false (L1523-L1525);
- converts `Some(EqlOutput::Store(ct))` to `Some(EqlOutput::Query(ct.into_query_operand()))` (L1528-L1530);
- leaves `Some(Query(_))` and `None` as they are (L1531).

This runs after the merge, so an inbound `Store` payload bound at a query-operand position is projected the same way as a locally encrypted one.
`into_query_operand` is in `cipherstash_client` (source not read).

`query_operand` is `false` for every output param of a statement whose `output_params` were not replaced from a `ParamPlan` (frontend.rs L939-L947); replacement happens only inside `if typed_statement.requires_transform()` and only when `transform_statement` returns `Some` (L839, L849-L865).

### 3.8 Metrics and return (L1130-L1149)

`duration` spans L1113-L1130: encrypt, merge, and projection, but not inbound parsing or plaintext conversion.
`encrypted_count` counts `Some` entries after the merge, so inbound payloads are counted (L1133).
`with_metrics_scope` is a no-op when `session_id` is `None` (context/mod.rs L1928-L1931) and a no-op when the id is not in `statement_metrics` (L1870-L1872); it takes the `protocol_state` write lock and fails only when the lock is poisoned (L1866-L1869).
It sets `metadata.encrypted = true` unconditionally, including when `encrypted_count` is 0 (L1138).
Prometheus counters are gated on `prometheus_enabled()` (L1143-L1147).

## 4. What the caller does with the result

`bind.rewrite(&statement.output_params, encrypted)` (rewrite/bind.rs L261-L298) is the consumer, and several properties of `encrypt_params`' output only become wire behaviour there:

- Positional path (L266-L276): taken when `output_params.len() == param_values.len()` and every source is `Input(idx)` at its own index.
- Rebuild path (L278-L297): each output takes a clone of the param at `source.primary_input()`; a missing one fails with `MissingBoundParameter` (L281-L286). `param_values` is replaced by exactly `output_params.len()` entries (L293-L295).
- `apply_output` (L310-L321): an output with a column and no ciphertext is rewritten to NULL (L315-L318). An output with no column and no ciphertext is left byte-for-byte (L323, L341).
- `apply_encrypted` (L323-L345): a `Query(Selector)` is written as bare text (L333-L335); every other payload is written as JSON, prefixed with `0x01` when the param is binary format (L337-L340; `BindParam::rewrite`, L457-L467).

Consequences that tie back to `encrypt_params`:

- The `Ok(None)` produced at bind.rs L100-L102 for an unbound `Input` param is turned into an error only by `rewrite`: the lengths differ, the rebuild path runs, and `get(input)` fails. `encrypt_params` itself does not fail on it.
- An unbound or NULL *path step* of a fused selector produces `None` in `encrypt_params` and NULL on the wire (bind.rs L315-L318), not an error, provided the primary input exists.
- When the client binds more params than `output_params` and the plan is otherwise positional, the length check at L266 fails, the rebuild path runs, and the extra trailing params are not emitted.

On `Err`, `bind_handler` returns without calling `finish_bind`; `Frontend::intercept` calls `refuse_bind` and substitutes an error for the Bind (frontend.rs L288-L320).

## 5. Invariants

1. `inbound`, `skip`, `plaintexts`, `output_param_columns`, and the returned `encrypted` all have length `statement.output_params.len()` and share its ordering. Evidence: each is a `map` over `output_params` (bind.rs L133-L135, L78-L81; frontend.rs L1096, L1105-L1109) and `Context::encrypt` enforces the output length (context/mod.rs L1652-L1656, L1667-L1673).
2. A position has a plaintext only if its `OutputParam.column` is `Some` (bind.rs L85-L88), and `ZeroKms::encrypt` encrypts only positions where plaintext and column are both `Some` (zerokms.rs L260).
3. A position is never both locally encrypted and inbound: `skip` suppresses plaintext conversion wherever `inbound` is `Some` (frontend.rs L1096; bind.rs L82-L84).
4. An inbound `Store` payload reaches the output only after decryption under the current cipher with matching embedded keyset id, re-encryption for the destination column, a `Store`-shaped derived result, and SEM term equality (frontend.rs L1185-L1238; zerokms.rs L401-L408).
5. An inbound `Query` payload reaches the output with structural validation only (frontend.rs L1165-L1167; inbound_eql.rs L127-L180).
6. After L1126-L1128, no position with `query_operand == true` holds `EqlOutput::Store` (frontend.rs L1527-L1531).
7. `bind` is not modified by this function (L1092 takes `&Bind`).
8. A result shorter or longer than the request from the encryption service is rejected (context/mod.rs L1667-L1673; zerokms.rs L502-L508).

## 6. Assumptions and where each is established

| # | Assumption | Established by |
|---|---|---|
| A1 | `statement.output_params` describes the SQL PostgreSQL holds for this prepared-statement name. | Parse stores the Statement (frontend.rs L876) and `begin_bind` looks it up by name (context/lifecycle.rs L249). Nothing in `encrypt_params` or `bind_handler` re-checks it against the upstream statement. |
| A2 | Every input index in `output_params` (primary inputs and path-step params) is within the client's bound params. | Not established in this function. Missing primary inputs yield `None` (bind.rs L100-L102, L142-L144, L179-L181) and are rejected only by `Bind::rewrite` L281-L286. Missing path-step params yield `None` (bind.rs L242-L245) and nothing rejects them; the output is bound NULL (bind.rs L315-L318). |
| A3 | The number of params the client binds equals the number the statement declares. | Nothing found in this function. `Bind::rewrite` L266 compares lengths only to choose a path; surplus params are dropped by the rebuild path (L278-L295). |
| A4 | `statement.postgres_param_types[i]` is the true wire type of bound param `i`. | Client-supplied at Parse (frontend.rs L829-L830). Used unverified at bind.rs L91 and L401-L406; decoding failure is the only check (bind.rs L112-L114). |
| A5 | A binary param whose first byte is `0x01` is jsonb with a version header. | Nothing found; bind.rs L148 tests the byte without consulting the type OID. |
| A6 | A JSON object with keys `v`, `i` and one of `c`/`h`/`sv` (or the query-shaped key sets) is an EQL payload and not application plaintext. | Shape test only, inbound_eql.rs L57-L72. A storage-shaped object that is not valid EQL fails (L61-L62) and does not fall back to plaintext. |
| A7 | A 32-character lowercase-hex string at a `JsonAccessor`/`JsonPath` query-operand position is a tokenized selector. | Format test only, inbound_eql.rs L36-L48, L87-L92; stated as intentional at L11-L13. |
| A8 | `OutputParam.query_operand` is accurate for every param bound in a predicate. | `output_params_from_plan` copies it from the mapper plan (context/statement.rs L158). For a Statement that kept its default output params it is always `false` (frontend.rs L945). Whether such a statement can carry an encrypted predicate param is not established here. |
| A9 | The effective keyset is the same for the three reads made during one call (encrypt, inbound decrypt, verification re-encrypt). | Nothing found. Each is a separate `protocol_state.read()` (context/mod.rs L1638-L1644) separated by `.await` points (frontend.rs L1118, L1188, L1210). |
| A10 | The keyset in effect at Bind is the keyset the value must be encrypted under when the portal executes. | Nothing found in this function; the keyset is read at Bind (context/mod.rs L1658) and not recorded with the returned values. |
| A11 | `Column.config` and `Column.eql_term` captured at Parse still describe the destination column at Bind. | Captured at Parse by `get_column` (column_mapper.rs L166-L188). Not re-read at Bind. CONTEXT.md describes this as the stale-route case. |
| A12 | The encryption service returns ciphertexts in the order of the prepared plaintexts. | Count only is checked (zerokms.rs L502-L508; context/mod.rs L1667). Ordering rests on `encrypt_eql_v3` in `cipherstash_client`. |
| A13 | A SteVec-term column always has a SteVec SEM term configured when its `eql_term` is a JSON query variant. | Not enforced in `ZeroKms::encrypt`; absence falls back to `EqlOperation::Store` (zerokms.rs L277, L288, L299, L316). |
| A14 | The caller applies the returned vector with the same `output_params` slice. | `bind_handler` L1047 passes `&statement.output_params` from the same `Statement`. |
| A15 | Plaintext conversion returns `Err` rather than panicking for every column type. | Not held for `ColumnType::Timestamp`: from_sql.rs L381-L383 and L506 are `unimplemented!`. Whether a `Timestamp` column config can reach this path is not established here. |
| A16 | `EqlCiphertext::into_query_operand` removes the decryptable ciphertext and keeps the SEM terms. | External (`cipherstash_client`); stated in the doc comment at frontend.rs L1514-L1518 only. |

## 7. Callees

| Callee | Kind | What the caller depends on |
|---|---|---|
| `Bind::inbound_eql` (rewrite/bind.rs L129) | internal | Per-output classification; `Some` only for column-bearing, `Input`-sourced, bound, non-NULL params. |
| `inbound_eql::parse` (inbound_eql.rs L31) | internal | Shape classification and structural validation; fails closed on malformed storage-shaped and on query-shaped payloads outside query positions. |
| `Bind::to_plaintext_skipping` (bind.rs L72) | internal | Length equals `output_params.len()`; `None` for native, skipped, NULL, or unbound positions. |
| `bind_param_from_sql`, `bind_param_json_value`, `json_value_selector_plaintext`, `compose_json_selector_path` (data/from_sql.rs) | internal | Typed decoding per column cast type and term; rejects non-scalar JSON equality values (L181-L189). |
| `Context::encrypt` (context/mod.rs L1647) | internal | Reads the effective keyset; enforces output length. |
| `EncryptionService::encrypt` -> `ZeroKms::encrypt` (zerokms.rs L240) | internal | Store/Query operation choice from `eql_term`; positional placement. |
| `ZeroKms::init_cipher` (zerokms.rs L155) | internal | Cipher scoped to the connection keyset or the configured default (L178-L181). |
| `encrypt_eql_v3`, `ScopedCipher::init`, `ScopedCipher::decrypt` | external-black-box (source not read) | Output order and count; keyset scoping; AEAD authentication on decrypt. |
| `Frontend::merge_inbound_eql` (frontend.rs L1156) | internal | Authentication and SEM verification of inbound `Store` payloads; pass-through of inbound `Query` payloads. |
| `Context::decrypt_inbound_eql` -> `ZeroKms::decrypt_eql` (zerokms.rs L388) | internal | Keyset-id match and all-entries authentication. |
| `inbound_eql::sem_terms_match` (inbound_eql.rs L195) | internal | Equality of all searchable metadata between inbound and derived payloads. |
| `project_query_operand` (frontend.rs L1522) | internal | No `Store` payload remains at a query-operand position. |
| `EqlCiphertext::into_query_operand` | external-black-box (source not read) | Strips ciphertext, keeps SEM terms. |
| `Context::with_metrics_scope` (context/mod.rs L1920) | internal | Metrics only; write lock on `protocol_state`. |
| `Context::prometheus_enabled` (context/mod.rs L1806) | internal | Config gate. |

## 8. Shared state touched

- Read: `ConnectionProtocolState.settings` (effective keyset) through `Context::keyset_identifier`, up to three times per call.
- Read/write: process-wide `ZeroKms.cipher_cache` through `init_cipher` (zerokms.rs L162, L200-L202).
- Write: `ConnectionProtocolState.statement_metrics[session_id]` (`phase_timing`, `metadata.encrypted`, encrypted value count) at frontend.rs L1134-L1140.
- Write: Prometheus counters and histogram (L1120, L1144-L1146).
- Outbound: ZeroKMS requests for cipher initialisation, encryption, and inbound decryption.

## 9. Open questions

- Unclear whether Backend message handling runs between the `.await` points of one `encrypt_params` call and so changes the effective keyset between the reads at context/mod.rs L1658 and L1689; need to inspect the scheduling in `postgresql/driver.rs` and pg-proto.
- Unclear whether a statement with an encrypted predicate param can keep the default `output_params` with `query_operand: false` (frontend.rs L939-L947), i.e. whether `requires_transform()` is always true for it; need to inspect `packages/eql-mapper/src/type_checked_statement.rs` L150-L155 and the transformation rules.
- Unclear whether `EqlCiphertext::into_query_operand` removes every decryptable field for both scalar and SteVec payloads; need to inspect `cipherstash_client::eql`.
- Unclear whether `encrypt_eql_v3` preserves input order; need to inspect `cipherstash_client`.
- Unclear whether a column encrypt config can carry `ColumnType::Timestamp` and reach the `unimplemented!` arms at data/from_sql.rs L381-L383 and L506; need to inspect `proxy/encrypt_config/from_domain.rs`.
- Unclear whether the mapper guarantees a SteVec SEM term for every column typed with a JSON query term, which decides whether the `unwrap_or(EqlOperation::Store)` fallbacks at zerokms.rs L277-L316 are reachable; need to inspect `proxy/schema/eql_domains.rs` and the mapper's trait bounds.
- Unclear what PostgreSQL does with a Bind whose param count was reduced by the rebuild path in `Bind::rewrite` when the client bound surplus params; need to inspect `rewrite_parse_param_types` (frontend.rs ~L1480-L1509) and the upstream statement's declared param count.
- Unclear whether `MAPPER`/`PROTOCOL` debug targets are enabled in any shipped configuration, which decides whether the plaintext log at frontend.rs L1111 is emitted; need to inspect `log` module and `LogConfig` defaults.
