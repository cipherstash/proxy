## `Backend::check_column_config` in packages/cipherstash-proxy/src/postgresql/middleware/backend.rs (L569-L600)

Paths are relative to `packages/cipherstash-proxy/src/` unless they start with `packages/`.
`backend.rs` means `postgresql/middleware/backend.rs`.
`data_row.rs` means `postgresql/rewrite/data_row.rs`.
"Dossier 02" means `audit-context/functions/02-Backend-flush_encrypted_rows.md`.

Signature (backend.rs L569-L573):

```rust
fn check_column_config(
    &mut self,
    projection_columns: &[Option<Column>],
    ciphertexts: &[Option<EqlCiphertext>],
) -> Result<(), Error>
```

**Purpose:** Compares the table/column identifier embedded in a parsed ciphertext (`ct.identifier()`) with the identifier of the projection column the Proxy's mapper assigned to that result position.
It is the only comparison on the read path between a stored value's claimed origin and the column it was projected from.
`ZeroKms::decrypt_eql` builds its decrypt records from `payload.ciphertext` and `key_header.record_with_selector(..)` and passes `DecryptOptions::default()` (proxy/zerokms/zerokms.rs L424, L433-L435, L449); no identifier or expected descriptor from the projection column is passed there.

The function pairs its two slices with a plain `zip` (L574).
`projection_columns` has one entry per result column of one row.
`ciphertexts` is the flattened vector over every buffered row (backend.rs L505-L508).
The zip ends at the shorter side, so the number of positions examined is `min(projection_columns.len(), ciphertexts.len())`.

---

**Inputs & Assumptions:**

- `&mut self`: no field of `self` is read or written between L574 and L599. The receiver is unused.
- `projection_columns: &[Option<Column>]`.
  Source: `portal.projection_columns()` at backend.rs L502, which returns `&statement.projection_columns` for `Portal::Encrypted` (postgresql/context/portal.rs L46).
  Built on the frontend path by `ColumnMapper::get_projection_columns`, one entry per `typed_statement.projection.columns()` (postgresql/column_mapper.rs L38-L57), stored into `Statement` at frontend.rs L915 and L949-L955.
  `Some(Column)` only for a projection entry whose mapper type is `Value::Eql` and whose identifier is present in the encrypt config (column_mapper.rs L41-L52, L166-L188); a missing config is an error there, not `None` (column_mapper.rs L189-L200).
  Trust: derived from the client's SQL and the schema/encrypt-config snapshot in effect at Parse or Query time.
- `ciphertexts: &[Option<EqlCiphertext>]`.
  Source: `rows.iter_mut().flat_map(|row| data_row::as_ciphertext(row, projection_columns))` (backend.rs L505-L508).
  Content trust: each `Some` was deserialised by serde from stored column bytes (data_row.rs L105-L112), so every field including the `i` identifier is chosen by whoever wrote the stored row.
  Length: `sum over rows of min(row.columns.len(), projection_columns.len())` (data_row.rs L21, L43).
- `Column.identifier: Identifier` (postgresql/context/column.rs L9), where `Identifier` is `cipherstash_client::eql::Identifier` (lib.rs L23-L26).
  Constructed as `Identifier::new(table.value.to_string(), column.value.to_string())` from the mapper's `TableColumn` (column_mapper.rs L42-L44).
  The identifier has no schema component at this construction site; it carries the two strings read at backend.rs L579-L580 as `.table` and `.column`.

Preconditions, and what establishes each:

| Precondition | Established by |
|---|---|
| `ciphertexts[k]` for `k < projection_columns.len()` is the value from result column `k` of one row | `as_ciphertext` output order (data_row.rs L21, L43) together with the first row having `columns.len() >= projection_columns.len()`. The second half: nothing found. |
| Every row's ciphertexts are examined | nothing found. L574 is the only iteration and it ends at `projection_columns.len()`. |
| A `Some` ciphertext appears only where the aligned projection entry is `Some` | `as_ciphertext`: the `column_config.as_ref()` chain yields `None` for a `None` config (data_row.rs L22-L25). Holds for aligned positions only. |
| `projection_columns.len()` equals the row width PostgreSQL returned | nothing found (see dossier 02, Block 4; `row_description_handler` zips without a length check at backend.rs L683). |
| `Identifier` equality means same table and same column | external black box (`cipherstash_client::eql::Identifier: PartialEq`); source not on disk. |
| `ct.identifier()` returns the payload's `i` field | external black box; the only in-repo evidence is the fixture assertions at data_row.rs L243-L246 and L283-L286, where a payload with `"i": {"c": "encrypted_text", "t": "encrypted"}` compares equal to `Identifier::new("encrypted", "encrypted_text")` (data_row.rs L190, L235). |
| The projection column's identifier is the name the stored value was encrypted under | nothing found. The encrypt config moves an entry to a new identifier on rename (proxy/encrypt_config/config.rs L64-L69, L75-L86); stored payloads keep the `i` they were written with. |

---

**Outputs & Effects:**

- `Ok(())`: every examined pair was one of `(Some, Some)` with equal identifiers, `(Some, None)`, or `(None, None)` (L576-L588, L599).
- `Err(Error::Encrypt(EncryptError::ColumnConfigurationMismatch { table, column }))` from the first examined pair that fails (L578-L582, L591-L595). The loop returns at the first failure; later pairs are not examined.
- No state writes. No metrics. No logging. No external calls beyond `ct.identifier()` and `Identifier` equality.
- Strings placed in the error:
  - `(Some(col), Some(ct))` arm: `col.identifier.table` and `col.identifier.column`, i.e. the configured name (L579-L580).
  - `(None, Some(ct))` arm: `ct.identifier().table` and `ct.identifier().column`, i.e. strings deserialised from stored bytes (L592-L593).
- How the error leaves the Proxy: `?` at backend.rs L512 returns it from `flush_encrypted_rows`; both callers pass it to `decryption_failure` (backend.rs L247, L298), which builds one `ErrorResponse` through `error_response` -> `error_to_response` (backend.rs L114-L116, L127).
  `ColumnConfigurationMismatch` has no dedicated arm in `error_to_response` and takes the catch-all `diagnostics::system_error(err.to_string())` (postgresql/error_handler.rs L73).
  The display string interpolates `{column}` and `{table}` (error.rs L334-L335), so the table and column strings reach the client verbatim.
- `DECRYPTION_ERROR_TOTAL` is not incremented on this error; the `inspect_err` that increments it is attached to `decrypt` only (backend.rs L522-L524).
- On `Err`, the batch taken at backend.rs L475 is dropped whole; no row of it is emitted (dossier 02, Outputs & Effects).

---

**Block-by-Block:**

### Block 1: the zip

```rust
// L574
for (col, ct) in projection_columns.iter().zip(ciphertexts) {
```

- **What:** Iterates pairs `(projection_columns[k], ciphertexts[k])` for `k` in `0..min(P, C)`, where `P = projection_columns.len()` and `C = ciphertexts.len()`.
- **Assumes:** Position `k` of the flat `ciphertexts` slice corresponds to projection column `k`. That is true for flat position `k` only while `k` is inside the first row's contribution.
- **Shape of `ciphertexts`:** row `j` contributes `min(w_j, P)` entries, where `w_j = rows[j].columns.len()` (data_row.rs L21). Entry `i` of that contribution was parsed under `projection_columns[i]`.
- **Case `w_0 >= P` (first row at least as wide as the projection):** the first row contributes exactly `P` entries. The zip consumes exactly those. Pairs are aligned. Rows `1..n` contribute flat positions `P..C`, none of which the zip reaches.
- **Case `w_0 < P` (first row narrower than the projection):** the first row contributes `w_0` entries. The zip continues into the second row's contribution: `projection_columns[w_0 + i]` is paired with the ciphertext that `as_ciphertext` parsed from row 1, column `i`, under `projection_columns[i]`. Pairs from position `w_0` on are not aligned. Nothing in this function or in `flush_encrypted_rows` (backend.rs L491-L512) compares `w_0` with `P`.
- **Case `P == 0`:** the zip is empty and the function returns `Ok(())` at L599. `as_ciphertext` also returns an empty vector for every row in that case (data_row.rs L21). An `Encrypted` portal with an empty projection is constructible on the simple-query path, where `Portal::encrypted` is assigned for every `Route::Map` statement with no `has_projection` guard (frontend.rs L531), in contrast to the Bind path (frontend.rs L1049).
- **Case `C < P`:** with one row narrower than the projection, the zip ends at `C`.
- **Number of rows examined per call:** one, in the aligned case. A batch holds up to 4096 rows or 64 MiB (backend.rs L75-L76, L287-L288), and each `flush_encrypted_rows` call makes one `check_column_config` call (backend.rs L512). An execution that spans several flushes has the first row of each batch examined.
- **Which row is first:** `encrypted_rows` is filled by `push` in arrival order (backend.rs L286) and taken whole (backend.rs L475), so the examined row is the first DataRow PostgreSQL sent for that operation since the previous flush. Its identity is determined by the statement's row order and by stored data.

### Block 2: `(Some(col), Some(ct))`

```rust
// L576-L584
(Some(col), Some(ct)) => {
    if &col.identifier != ct.identifier() {
        return Err(EncryptError::ColumnConfigurationMismatch {
            table: col.identifier.table.to_owned(),
            column: col.identifier.column.to_owned(),
        }.into());
    }
}
```

- **What:** Requires the ciphertext's identifier to equal the projection column's identifier.
- **Compared:** `Identifier` only. `Column.config`, `Column.postgres_type`, and `Column.eql_term` (postgresql/context/column.rs L10-L12) are not read. The ciphertext's version, keyset id, and record descriptor are not read.
- **Contrast with the write-side sibling:** `validate_storage_metadata` checks version, identifier, and additionally requires the record descriptor to equal `"{table}/{column}"`, with a comment that the descriptor is covered by the record's AEAD tag "unlike the self-reported `i` field alone" (postgresql/inbound_eql.rs L98-L114). That function runs per value. The read path has no descriptor comparison in the Proxy.
- **Assumes:** `ct.identifier()` reflects where the value was stored. The value is whatever serde produced from the stored JSON (data_row.rs L112); for a payload reshaped from a JSON entry, `i` is left in place by `json_entry_into_ste_vec_document`, which moves only `s`, `c`, `a`, `op` (data_row.rs L141-L145).
- **Establishes:** For an examined position, the payload's self-declared `i` equals the configured identifier. It establishes nothing about the encrypted record inside the payload.
- **Reaches the error with consistent data when:** the config identifier and the stored `i` differ for a reason other than misplacement. Two producers found: the non-aligned zip of Block 1 (`w_0 < P`), and an encrypt-config entry moved by `rename_column` / `rename_table` (proxy/encrypt_config/config.rs L64-L69, L75-L86) while stored payloads retain the earlier name.

### Block 3: `(Some(_), None)`

```rust
// L585-L586
// configured column with NULL ciphertext
(Some(_), None) => {}
```

- **What:** Accepts a configured column with no ciphertext.
- **The comment names one producer; `as_ciphertext` has two** (data_row.rs L22-L42):
  - the column is SQL NULL (`.filter(|_| data_column.is_some())`, L24);
  - the column has bytes and `to_eql_ciphertext` failed; the failure is logged and `.ok()` converts it to `None` (L27-L41).
- **Consequence for what the check covers:** a configured column whose bytes are not a deserialisable EQL payload is indistinguishable here from NULL and is accepted. Its original bytes are later forwarded unchanged, because `data_row::rewrite` leaves a column untouched when its plaintext is `None` (data_row.rs L57-L62; dossier 02, Block 9).

### Block 4: `(None, None)`

```rust
// L587-L588
// unconfigured column *should* have no ciphertext,
(None, None) => {}
```

- **What:** Accepts an unconfigured column with no ciphertext.
- **Why it always holds for aligned input:** `as_ciphertext` never inspects the bytes of a column whose config is `None` (data_row.rs L22-L23). The `None` ciphertext states that parsing was not attempted, not that the bytes are not an EQL payload.

### Block 5: `(None, Some(ct))`

```rust
// L589-L596
// ciphertext with no column configuration is bad
(None, Some(ct)) => {
    return Err(EncryptError::ColumnConfigurationMismatch {
        table: ct.identifier().table.to_owned(),
        column: ct.identifier().column.to_owned(),
    }.into());
}
```

- **What:** Rejects a ciphertext at a position whose projection entry is `None`.
- **Producers:** none for aligned input, since `as_ciphertext` yields `Some` only under a `Some` config (data_row.rs L22-L25). The arm is reachable only through the non-aligned zip of Block 1 (`w_0 < P`): `projection_columns[w_0 + i]` is `None` while row 1, column `i` parsed to `Some` under `projection_columns[i]`.
- **Error content:** the table and column strings are taken from the ciphertext, i.e. from stored bytes (L592-L593), and reach the client through the catch-all arm at postgresql/error_handler.rs L73.

### Block 6: return

```rust
// L599
Ok(())
```

- **Establishes for the caller:** the examined prefix of `ciphertexts` has no identifier disagreement. It does not establish the same for flat positions `>= projection_columns.len()`.

---

**Cross-Function Dependencies:**

Callees:

- `EqlCiphertextV3::identifier()` (external black box, cipherstash-client 0.42.2 per `Cargo.toml` L50; source not located on disk). Called at L577, L592, L593. Sent: nothing beyond `&self`. Assumed: returns a reference to the payload's `i` field for both the `Encrypted` and `SteVec` variants, without side effects and without panicking. Outcomes not excluded: a variant-dependent or defaulted identifier.
- `<Identifier as PartialEq>::ne` (external black box, same crate). Called at L577. Assumed: exact comparison of table and column. Outcomes not excluded: comparison that ignores or adds a field, or case/quoting normalisation that differs from how `column_mapper.rs` L42-L44 builds the configured identifier.
- `String::to_owned`, `From<EncryptError> for Error` (L579-L582, L592-L595): plain construction.

Callee of the caller that shapes this function's input (read in full because the zip depends on it):

- `data_row::as_ciphertext` (internal, data_row.rs L16-L47): fixes the length and alignment of `ciphertexts`. Returns `min(row width, projection length)` entries per row; yields `Some` only under a `Some` config with deserialisable bytes; converts every parse failure to `None`; returns `Vec`, not `Result`. The `ColumnIsNull` arm at data_row.rs L28-L32 has no producer in `to_eql_ciphertext` (data_row.rs L94-L113 constructs `ColumnCouldNotBeParsed` or a serde error only).

Callers:

- `Backend::flush_encrypted_rows`, backend.rs L512, the only call site (repository search for `check_column_config` returns L512 and L569 only).
  It passes the per-row `projection_columns` (L502) and the all-rows flat `ciphertexts` (L505-L508).
  It treats `Ok(())` as covering the whole batch: the full `ciphertexts` vector, not the examined prefix, is moved into `self.context.decrypt(ciphertexts)` at L522.
  The check runs after `start = Instant::now()` (L510), so its time is inside the recorded decrypt duration (L526).
- No test calls this function or asserts `ColumnConfigurationMismatch`: a search of `packages/` for the variant returns the definition (error.rs L335), the cipherstash-client conversion (error.rs L474-L475), and the two construction sites here (L578, L591).

Shared state:

- None read or written by this function.
- Indirect: `Statement.projection_columns` behind `Arc<Statement>` in the portal (postgresql/context/portal.rs L9, L46), immutable for the duration of the call.

Invariant couplings:

- `check_column_config` and `as_ciphertext` both zip against the same `projection_columns`, the first once and the second once per row. The check's coverage equals `as_ciphertext`'s first-row output only when `w_0 >= P`.
- `row_description_handler` advertises type OIDs from `col.postgres_type` for the same positions (backend.rs L674-L688). The plaintext is later encoded by `Plaintext` variant, not by `Column` (dossier 02, Block 9). The identifier comparison here is the only step that ties a decrypted value to the `Column` whose type was advertised, and it runs on the examined prefix only.
- For a multi-statement simple Query, one portal is registered for the operation and it holds the last `Route::Map` statement's `projection_columns` (frontend.rs L476, L480-L531, L568-L569). `check_column_config` receives that statement's columns for the rows of whichever statement produced them.
- Read path vs write path: the inbound check is per value and includes the AEAD-covered descriptor (postgresql/inbound_eql.rs L94-L114); the outbound check is per batch prefix and covers the self-reported `i` only (L577).

---

**Open Questions:**

- unclear; need to inspect cipherstash-client 0.42.2 `eql::Identifier` (`PartialEq` derive or manual impl, field set) to determine exactly what L577 compares.
- unclear; need to inspect cipherstash-client 0.42.2 `EqlCiphertextV3::identifier()` for both variants to confirm it returns the deserialised `i` field and cannot panic.
- unclear; need to inspect cipherstash-client `ScopedCipher::decrypt` with `DecryptOptions::default()` to determine whether the record descriptor (`"{table}/{column}"`, postgresql/inbound_eql.rs L107-L111) is authenticated at decrypt and whether anything compares it with an expected column; the Proxy passes no expected descriptor on the read path (proxy/zerokms/zerokms.rs L424, L433-L435, L449).
- unclear; need to inspect eql-mapper's `projection.columns()` (wildcards, `RETURNING`, set operations, schema drift after Parse) to determine when `projection_columns.len()` exceeds the first DataRow's width, which is the condition for the non-aligned zip and for the `(None, Some)` arm.
- unclear; need to inspect eql-mapper's `table_column()` name resolution (aliases, schema-qualified names, quoted/mixed-case identifiers, views) to determine whether `Identifier::new(table.value.to_string(), column.value.to_string())` at column_mapper.rs L42-L44 always equals the `i` written at encrypt time for the same column.
- unclear; need to inspect `SchemaMiddleware::apply_ddl` callers of `EncryptConfig::rename_column` / `rename_table` (proxy/encrypt_config/config.rs L64-L86) to determine whether stored payloads' `i` is expected to be rewritten after a rename, or whether the first-row comparison at L577 then fails for every pre-rename row.
- unclear; need to inspect `diagnostics::system_error` to determine whether the table/column strings taken from stored bytes at L592-L593 are length-limited or sanitised before being placed in the ErrorResponse.
