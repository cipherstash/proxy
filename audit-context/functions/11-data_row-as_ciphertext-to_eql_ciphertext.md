## `data_row::as_ciphertext` / `to_eql_ciphertext` in packages/cipherstash-proxy/src/postgresql/rewrite/data_row.rs (L16-L47, L94-L156)

Paths are relative to `packages/cipherstash-proxy/src/` unless they start with `packages/`.
`data_row.rs` means `postgresql/rewrite/data_row.rs`; `backend.rs` means `postgresql/middleware/backend.rs`; `zerokms.rs` means `proxy/zerokms/zerokms.rs`.
Line numbers without a file name refer to `data_row.rs`.

Signatures:

- `pub fn as_ciphertext(row: &mut DataRow, column_configuration: &[Option<Column>]) -> Vec<Option<EqlCiphertext>>` (L16-L19).
- `fn to_eql_ciphertext(column: &Option<Bytes>) -> Result<EqlCiphertext, Error>` (L94).
- Helpers analysed with them: `is_json_entry` (L121-L123), `json_entry_into_ste_vec_document` (L133-L151), `log_deserialise_error` (L153-L156).

**Purpose:** `as_ciphertext` converts one DataRow into a positional vector of optional EQL ciphertexts, one per zipped (row column, projection column) pair.
`to_eql_ciphertext` is the only place bytes returned by PostgreSQL are parsed into `EqlCiphertext` (`cipherstash_client::eql::EqlCiphertextV3`, lib.rs L23-L24) on the read path.
The vector it returns is the sole input to `check_column_config` (backend.rs L512) and to `Context::decrypt` (backend.rs L522); a `None` at a position is what later causes `data_row::rewrite` to leave that column's stored bytes in the outgoing row (L57-L62).

---

**Inputs & Assumptions:**

- `row: &mut DataRow` (pg-proto type, `columns: Vec<Option<Bytes>>` as constructed at L186). Trust: semi-trusted to untrusted. Column bytes were written by any party able to write the underlying rows; the column count comes from PostgreSQL's framing as decoded by pg-proto (source not on disk).
- `column_configuration: &[Option<Column>]`. Trust: derived inside the Proxy. It is `portal.projection_columns()` (backend.rs L502, L507), which is `statement.projection_columns` (context/portal.rs L46), built at Parse/Query time by `ColumnMapper::get_projection_columns` with one entry per `typed_statement.projection.columns()` (column_mapper.rs L36-L59).
- Implicit: none. Neither function reads connection state, the portal's format codes, the keyset, or a clock.
- Logging: `debug!`/`error!` on target `DECRYPT` (L29, L38, L154).

Preconditions, and what establishes each:

| Precondition | Established by |
|---|---|
| `row.columns.len() == column_configuration.len()` | nothing found. L21 zips without comparing lengths; the caller (backend.rs L505-L508) does not compare either. |
| Position `i` of the row and position `i` of `column_configuration` denote the same result column | nothing found in these functions or the caller. Rests on the mapper's projection order (column_mapper.rs L38-L57) matching PostgreSQL's output. |
| A column whose configuration is `Some` holds a jsonb-domain value (text JSON, or `0x01` + JSON) | nothing found. The format is decided from the first byte alone (L99-L103). |
| A root-level `"s"` key appears only on an `eql_v3_json_entry` result | nothing found. `is_json_entry` tests key presence only (L122). |
| The final deserialisation rejects any payload that is not a well-formed v3 scalar or SteVec document | `serde_json::from_value::<EqlCiphertextV3>` at L112 only; the type's `Deserialize` impl is in cipherstash-client 0.42.2, source not on disk. |

---

**Outputs & Effects:**

- `as_ciphertext` returns `Vec<Option<EqlCiphertext>>` of length `min(row.columns.len(), column_configuration.len())` (L21, L43). It returns no `Result`; no failure reaches the caller (L19).
- An entry is `Some` only when the aligned configuration is `Some`, the column is non-NULL, and `to_eql_ciphertext` returned `Ok` (L22-L42).
- An entry is `None` for four distinct causes that the return type does not distinguish: configuration `None` (L22-L23), column NULL (L24), trailing position cut by the zip (no entry at all, L21), and parse failure (L41).
- State writes: the only write to `row` is `*data_column = None` at L31, inside the `ColumnIsNull` arm. No path in `to_eql_ciphertext` constructs `ColumnIsNull` (see Block 2), so on every reachable path the row is returned unmodified.
- Logs: `error!` with the configured table and column names on any parse failure (L34-L38); `debug!` with the `serde_json` error string (L154).
- No external interaction, no metrics, no shared-state access.

---

**Block-by-Block:**

### Block 1: positional zip

```rust
// L20-L21
let mut result = vec![];
for (data_column, column_config) in row.columns.iter_mut().zip(column_configuration) {
```

- **What:** Walks row columns and projection columns in lockstep.
- **Assumes:** Both sequences have the same length and order.
- **Establishes:** `result.len() == min(row.columns.len(), column_configuration.len())`.
- **Does not establish:** one entry per row column. Row columns at index `>= column_configuration.len()` produce no entry and are not inspected. Configurations at index `>= row.columns.len()` are ignored.
- **Depended on by:** backend.rs L505-L508 flattens the per-row vectors into one; backend.rs L547 re-splits that flat vector with `chunks(result_column_count)`, where `result_column_count` is the first row's `columns.len()` (backend.rs L491-L492). The re-split matches the per-row boundaries only when every call here returns exactly `result_column_count` entries.

### Block 2: per-column decision and error swallowing

```rust
// L22-L43
let encrypted = column_config
    .as_ref()
    .filter(|_| data_column.is_some())
    .and_then(|config| {
        to_eql_ciphertext(data_column)
            .inspect_err(|err| match err {
                Error::Encrypt(EncryptError::ColumnIsNull) => {
                    debug!(target: DECRYPT, msg ="ColumnIsNull", ?config);
                    *data_column = None;
                }
                _ => {
                    let err = EncryptError::ColumnCouldNotBeDeserialised { table: ..., column: ... };
                    error!(target: DECRYPT, msg = err.to_string());
                }
            })
            .ok()
    });
result.push(encrypted);
```

- **What:** Parses a column only when it has a configuration and is non-NULL; converts any parse error to `None`.
- Paths:
  - Configuration `None`: `None` pushed (L22-L23). The bytes are never examined, so a column the mapper typed as native is not parsed even when its bytes are an EQL payload. As a result the `(None, Some(ct))` arm of `check_column_config` (backend.rs L590-L596) has no producer when its inputs are aligned.
  - Configuration `Some`, column `None` (SQL NULL): `None` pushed (L24). `to_eql_ciphertext` is not called.
  - Configuration `Some`, column `Some`, parse `Ok`: `Some(ciphertext)` pushed.
  - Configuration `Some`, column `Some`, parse `Err`: `inspect_err` runs, `.ok()` yields `None` (L41).
- **`ColumnIsNull` arm (L28-L32):** `to_eql_ciphertext` returns only `EncryptError::ColumnCouldNotBeParsed` (L96, L102, L138) or a `serde_json::Error` routed through `From<serde_json::Error> for Error` (error.rs L648-L651) into `EncryptError::CiphertextCouldNotBeSerialised` (error.rs L320). `ColumnIsNull` (error.rs L326) is constructed in this crate only from `cipherstash_client::eql::EqlError::ColumnIsNull` (error.rs L467), a conversion these functions do not call. The arm, and with it the only mutation of `row`, has no reachable producer.
- **Catch-all arm (L33-L39):** Builds `ColumnCouldNotBeDeserialised` from `config.identifier` (the projection column's table and column, not anything read from the payload), logs it at `error!`, and drops it. The original error value is not logged here; a `serde_json` error was already logged at `debug!` by `log_deserialise_error` (L154), while the three `ColumnCouldNotBeParsed` sites (L96, L102, L138) have no log line of their own.
- **Use of `config`:** `config` is read only for logging (L29, L35-L36). The column's `eql_term`, `postgres_type`, `cast_type` and identifier take no part in how the bytes are parsed or in which `EqlCiphertext` variant results.
- **Establishes:** `Some` entries exist only at positions with a `Some` configuration.
- **Does not establish:** that a `Some` configuration with non-NULL bytes yields `Some`. The failure case is indistinguishable, to the caller, from SQL NULL and from an unconfigured column.
- **Depended on by:**
  - `ZeroKms::decrypt_eql` skips `None` positions (zerokms.rs L440) and leaves their plaintext `None` (zerokms.rs L461).
  - `data_row::rewrite` overwrites a column only for a `Some` plaintext (L58-L62), so a position that failed to parse leaves the function `flush_encrypted_rows` with its stored bytes intact and no error (backend.rs L553-L564).
  - `check_column_config` accepts `(Some(_), None)` without distinction (backend.rs L586).

### Block 3: NULL and empty guards in `to_eql_ciphertext`

```rust
// L95-L103
let Some(bytes) = column else {
    return Err(EncryptError::ColumnCouldNotBeParsed.into());
};
let json = match bytes.first() {
    Some(&JSONB_BINARY_VERSION) => &bytes[1..],
    Some(_) => &bytes[..],
    None => return Err(EncryptError::ColumnCouldNotBeParsed.into()),
};
```

- **What:** Rejects `None` and zero-length values, then selects the JSON slice by first byte: `0x01` (L14) is dropped, any other byte is kept.
- **Why here:** `&bytes[1..]` at L100 is reached only when `first()` is `Some`, so the slice cannot be out of range.
- The `None` return at L96 is not reachable from `as_ciphertext`, which filters NULL columns at L24. `to_eql_ciphertext` is private (L94) and has one call site (L26).
- **Assumes:** A leading `0x01` means binary jsonb and anything else means text JSON. The portal's result format code for the column is not passed in and is not consulted. A text-format value that begins with byte `0x01` is treated the same as a binary-format value.
- **Assumes:** The value's wire type is jsonb (a domain over jsonb, L69-L75). No type OID is available to this function.
- **Establishes:** `json` is a non-empty-or-empty slice (a lone `0x01` gives an empty slice, which then fails at L106).

### Block 4: generic JSON parse

```rust
// L105-L106
let mut value: serde_json::Value =
    serde_json::from_slice(json).map_err(log_deserialise_error)?;
```

- **What:** Parses the whole slice into an owned `serde_json::Value` tree.
- **Callee `serde_json::from_slice`** (serde_json 1.0.151, Cargo.lock L4346-L4347; source not on disk): depended on to fail for anything that is not exactly one JSON value. Size, nesting depth and duplicate-key handling are whatever serde_json does; nothing in this file bounds them.
- **Input size:** No length check precedes the parse. The only bound upstream is the held-row budget in `intercept_backend`, which is tested after the row is pushed (backend.rs L280-L288), so a single row larger than the budget is still held and reaches this parse.
- **Callee `log_deserialise_error`** (L153-L156): logs `err.to_string()` at `debug!` and converts with `err.into()`, which is `Error::Encrypt(EncryptError::CiphertextCouldNotBeSerialised(..))` (error.rs L648-L651, L320). The logged string is produced by serde_json from stored bytes.
- **Establishes:** `value` is some JSON value. It is not yet known to be an object.

### Block 5: root `"s"` detection and reshape

```rust
// L108-L110
if is_json_entry(&value) {
    json_entry_into_ste_vec_document(&mut value)?;
}

// L121-L123
fn is_json_entry(value: &serde_json::Value) -> bool {
    value.get("s").is_some()
}

// L136-L150
let object = value.as_object_mut().ok_or(EncryptError::ColumnCouldNotBeParsed)?;
let mut entry = serde_json::Map::new();
for key in ["s", "c", "a", "op"] {
    if let Some(field) = object.remove(key) {
        entry.insert(key.to_owned(), field);
    }
}
object.insert("k".to_owned(), Value::String("sv".to_owned()));
object.insert("sv".to_owned(), Value::Array(vec![Value::Object(entry)]));
Ok(())
```

- **What:** When the root value has a key `"s"`, moves root keys `s`, `c`, `a`, `op` into a new object, sets root `k` to `"sv"`, and sets root `sv` to a one-element array holding that object.
- **Gate (L122):** Presence of the key only. The value of `"s"` is not inspected, so `"s": null` or a non-string satisfies it. No other key (`v`, `i`, `h`, `c`) is required for the gate.
- **Unreachable error (L136-L138):** `Value::get("s")` returns `Some` only for an object, so when L109 is entered `as_object_mut` succeeds. `json_entry_into_ste_vec_document` therefore returns `Ok` on every path reachable from L108.
- **Overwrites:** `object.insert` at L147 and L148 replaces an existing root `k` and an existing root `sv`. A stored payload that has both a root `s` and its own `sv` array has that array discarded and replaced by the single synthesised entry.
- **Moves:** A root `op` is moved into the entry (L141-L144). In the scalar shape `op` is a root-level field (fixture at L235); in the entry shape it is an entry field (L127-L131). After the reshape the root has no `c` and no `op`.
- **Retained:** Every other root key stays at the root (`v`, `i`, `h`, and any scalar index keys such as `bf`, `hm` that were present).
- **Assumes:** A root `s` identifies an `eql_v3_json_entry` produced by `eql_v3."->"` / `eql_v3.jsonb_path_query` (L115-L120). What establishes it: nothing found. The projection column's `eql_term` is available in `as_ciphertext` (`config`) but is not passed to `to_eql_ciphertext` (L26, L94).
- **Assumes:** The selector in `s` is the one the statement asked for. Nothing in these functions compares it with anything.
- **Establishes:** The variant of `EqlCiphertext` produced at L112 for an object with root `s` is decided by stored data: the object now carries `k: "sv"` and an `sv` array and no root `c`.
- **Depended on by:** `ZeroKms::decrypt_eql` branches on the variant. `Encrypted` sends `payload.ciphertext` as a scalar record (zerokms.rs L423-L426). `SteVec` takes the first entry only under `RootOnly` (zerokms.rs L427-L429, L472-L483), decodes its selector (zerokms.rs L431), and builds the record from `document.key_header.record_with_selector(entry.ciphertext, selector)` (zerokms.rs L432-L436). The synthesised one-entry `sv` makes the reshaped entry that "root" entry.

### Block 6: typed deserialisation

```rust
// L112
serde_json::from_value(value).map_err(log_deserialise_error)
```

- **What:** Deserialises the (possibly reshaped) value into `EqlCiphertextV3`.
- **Callee `EqlCiphertextV3: Deserialize`** (cipherstash-client 0.42.2, Cargo.lock L752-L753; source not on disk). Everything about which shapes are accepted is inside it: how the `Encrypted` and `SteVec` variants are told apart, whether `v` is checked against 3, whether unknown root keys are rejected, whether `h` is required for `SteVec`, and what `i` must contain.
- **Not checked here:** the payload identifier against the projection column, the payload keyset against the connection keyset, the payload version, and the SteVec entry count. The identifier comparison happens later in `check_column_config`, which zips `projection_columns` with the flat vector and so ends after `projection_columns.len()` entries (backend.rs L574). The keyset comparison in `decrypt_eql` runs only for `AllEntries` (zerokms.rs L401-L408); the read path uses `RootOnly` (zerokms.rs L373).
- **Establishes:** An `Ok` value is a structurally valid `EqlCiphertextV3` by that type's own rules, and nothing more.

---

**Cross-Function Dependencies:**

Callees:

- `to_eql_ciphertext` (internal, L94-L113): depended on by `as_ciphertext` to classify bytes. Returns `Err` on NULL (L96, unreachable from L26), empty value (L102), invalid JSON (L106), and shape rejected by `EqlCiphertextV3` (L112). Never returns `ColumnIsNull`.
- `is_json_entry` (internal, L121-L123): key-presence test on `"s"`; implies the value is an object.
- `json_entry_into_ste_vec_document` (internal, L133-L151): unconditional overwrite of root `k` and `sv`; its one error site is unreachable behind the L108 gate.
- `log_deserialise_error` (internal, L153-L156): debug log plus conversion into `EncryptError::CiphertextCouldNotBeSerialised`.
- `serde_json::from_slice`, `serde_json::from_value`, `Value::get`, `Value::as_object_mut`, `Map::remove`/`insert` (external; serde_json 1.0.151, source not on disk): sent the raw column bytes. Depended on to reject non-JSON input and to return, not abort, on deep or large input.
- `EqlCiphertextV3` deserialisation (external-black-box, cipherstash-client 0.42.2): sent a JSON value fully shaped by stored data and by the L133-L151 reshape. Depended on for all structural validation of the payload.
- `bytes::Bytes::first` / slicing (external): L99-L101.

Callers:

- `Backend::flush_encrypted_rows`, backend.rs L505-L508, is the only non-test caller. It treats the result as one entry per row column: it flattens across rows and later re-chunks by the first row's width (backend.rs L491-L492, L547). It also treats `None` as "nothing to decrypt here" and has no way to tell a parse failure from NULL.
- Tests at L229-L307 cover binary and text scalar payloads, a text SteVec document with `s` only inside `sv[]` (L275), and NULL columns. No fixture in this module has a root-level `s`, an empty value, invalid JSON, or a length mismatch between row and configuration.

Shared state:

- None read or written. The `&mut DataRow` is borrowed from the caller's local `rows` (backend.rs L475, L506).

Invariant couplings:

- Parse failure -> `None` here, `None` -> column untouched in `rewrite` (L57-L62): together these mean a configured column's stored bytes leave `flush_encrypted_rows` unchanged whenever they fail to deserialise, and the only signal is the `error!` log at L38.
- The same stored bytes reach the client under whatever type the RowDescription rewrite advertised for that position from the projection column (documented in `audit-context/functions/02-Backend-flush_encrypted_rows.md`, Invariant couplings); these functions do not see that type.
- Stored data, not statement typing, selects the decrypt path: root `s` -> SteVec variant -> `record_with_selector` (zerokms.rs L427-L436); otherwise the variant is whatever `EqlCiphertextV3` picks for the object as stored.
- Vector length `min(row width, projection length)` couples to the fixed-width chunking at backend.rs L547 and to the truncating zip in `check_column_config` (backend.rs L574).

---

**Open Questions:**

- unclear; need to inspect cipherstash-client 0.42.2 `EqlCiphertextV3` / `EncryptedPayloadV3` / `SteVecPayloadV3` serde attributes to determine how the two variants are discriminated (tag on `k`, untagged, or custom), and so what an object with root `c` but no `s`, or with both `sv` and `c`, deserialises to.
- unclear; need to inspect the same types for `deny_unknown_fields`: after the reshape, scalar index keys left at the root (`bf`, `hm`) either cause rejection or are silently dropped.
- unclear; need to inspect the same types to determine whether `v` is validated, whether `h` (key header) is required on a SteVec document, and whether `i` is constrained.
- unclear; need to inspect cipherstash-client `KeyHeader::record_with_selector` and `ScopedCipher::decrypt` to determine what binds an entry ciphertext to the `h` header, the selector `s`, the `i` identifier and the keyset, since these functions accept all four from stored bytes without comparison.
- unclear; need to inspect serde_json 1.0.151 (and the crate's enabled features in Cargo.toml, e.g. `unbounded_depth`, `arbitrary_precision`, `preserve_order`) for the recursion limit, duplicate-key behaviour of `Value` maps, and trailing-data handling in `from_slice`.
- unclear; need to inspect serde_json and cipherstash-client error `Display` output to determine whether the string logged at L154 can contain fragments of the stored column value.
- unclear; need to inspect pg-proto 0.12.0 `DataRow` decoding (source not on disk) for the maximum column length and whether a zero-length non-NULL column is delivered as `Some(empty)`, which is the input for L102.
- unclear; need to inspect eql-mapper `projection.columns()` to determine when its length differs from the DataRow width, which is the input condition for the L21 truncation.
- unclear; need to inspect `ColumnMapper::get_column` (column_mapper.rs L52) to determine which `EqlTerm` variants yield `Some(Column)` for a projection column, and whether a `JsonAccessor` (`->>`, advertised as TEXT in context/column.rs L81) result arrives in a jsonb-compatible wire form that L99-L103 handles.
- unclear; need to inspect `packages/cipherstash-proxy-integration/src/select/jsonb_get_field_as_ciphertext.rs` to see which root-`s` shapes are exercised end to end.
