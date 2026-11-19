## `Backend::flush_encrypted_rows` in packages/cipherstash-proxy/src/postgresql/middleware/backend.rs (L473-L567)

Paths are relative to `packages/cipherstash-proxy/src/` unless they start with `packages/`.
`backend.rs` means `postgresql/middleware/backend.rs`.

Signature: `async fn flush_encrypted_rows(&mut self) -> Result<Vec<BackendMessage>, Error>` (backend.rs L473).

**Purpose:** Turns the DataRows held in `Backend.encrypted_rows` into client-bound DataRows whose encrypted columns carry plaintext in the wire format the client asked for.
It is the only place stored column bytes are parsed as EQL ciphertext, sent to the encryption service, and written back into rows.
Every held row leaves the Proxy through this function or is dropped by it; there is no other emitter of a held row (backend.rs L286 is the only push, L475 the only take, L123 the only other clear).

---

**Inputs & Assumptions:**

- No parameters. Everything is implicit state.
- `self.encrypted_rows: Vec<DataRow>` (backend.rs L69). Content trust: semi-trusted to untrusted. Row bytes come from upstream PostgreSQL and were written by any party able to write the underlying rows. Row count and per-row column count come from PostgreSQL's framing.
- `self.encrypted_rows_operation: Option<OperationId>` (backend.rs L70). Set by `intercept_backend` at L271-L273 from pg-proto's correlation of the DataRow.
- `self.encrypted_rows_bytes` (backend.rs L71). Reset only; not read here.
- `Portal` reached through `Context::get_portal_from_execute(operation)` (context/mod.rs L1290-L1307). Built on the frontend path at Bind (frontend.rs L1050-L1054) or simple Query (frontend.rs L531). Carries `format_codes` (client-chosen at Bind, rewrite/bind.rs L387-L391) and `Arc<Statement>` whose `projection_columns` were computed at Parse/Query time from the EQL mapper's typed projection (frontend.rs L915, column_mapper.rs L32-L59).
- Connection keyset, read from `ConnectionProtocolState.settings` at flush time (context/mod.rs L1638-L1644, L1681). Client-selectable once PostgreSQL confirms the `SET`.
- `self.context.prometheus_enabled()` (operator config, context/mod.rs L1806-L1808).
- Clock: `Instant::now()` at L510 and L526.

Preconditions, and what establishes each:

| Precondition | Established by |
|---|---|
| `encrypted_rows` is non-empty | Caller: backend.rs L231 (`!self.encrypted_rows.is_empty()`), and L286 pushes immediately before the L294 call. |
| `encrypted_rows_operation` is `Some` whenever rows are held | Caller: backend.rs L266-L270 returns before the push when `operation` is `None`; L271-L273 stores it before L286. |
| All held rows belong to one operation | Caller: backend.rs L271-L279 calls `decryption_failure` when the stored operation differs from the incoming one. |
| The operation's portal was `Portal::Encrypted` when each row was held | `data_row_handler`, backend.rs L735-L745, which reads the same `get_portal_from_execute`. Re-checked here at L482-L488. |
| Every held row has the same column count | nothing found. Width is read from the first row only (L491-L492). |
| `projection_columns.len()` equals the row width PostgreSQL returns | nothing found. See Block 4. |
| `format_codes.len()` is 0, 1, or the row width | nothing found in the Proxy. `Bind::try_from` validates parameter format-code counts (rewrite/bind.rs L363-L373) and copies result format codes unchecked (L387-L391). |
| The row width is non-zero | nothing found. See Block 8. |
| `decrypt` returns exactly one entry per ciphertext passed | `ZeroKms::decrypt_eql` only (proxy/zerokms/zerokms.rs L445, L461). `Context::decrypt` does not check (context/mod.rs L1677-L1683). |

---

**Outputs & Effects:**

- `Ok(Vec<BackendMessage>)`: zero or more `BackendMessage::DataRow`. The count is `min(ceil(plaintexts.len() / result_column_count), rows.len())` (L547, L551-L565). Nothing compares it with the number of rows taken at L475.
- `Err(Error)`: on every error path the held rows are already gone from `self` (L475) and are dropped with the local `rows`; nothing is emitted from this call.
- State writes on every path, including errors: `encrypted_rows_operation = None` (L474), `encrypted_rows = []` (L475), `encrypted_rows_bytes = 0` (L476).
- State write on success path only: statement metrics decrypt duration (L529-L532).
- Metrics: `DECRYPTION_ERROR_TOTAL` on decrypt error (L522-L524); `DECRYPTION_REQUESTS_TOTAL`, `DECRYPTED_VALUES_TOTAL`, `DECRYPTION_DURATION_SECONDS` when Prometheus is enabled (L535-L544).
- External interaction: one `EncryptionService::decrypt` call (L522), which for `ZeroKms` reaches `init_cipher` and `cipher.decrypt` (zerokms.rs L400, L452-L455).
- Mutation of row content before decryption: `as_ciphertext` takes `&mut DataRow` (L506-L507) and can set a column to `None` (rewrite/data_row.rs L31).
- `discard_execution` is not written here. Both callers set it on `Err` (backend.rs L237, and L298 via `decryption_failure` L126).

---

**Block-by-Block:**

### Block 1: take the buffer

```rust
// L474-L476
let operation = self.encrypted_rows_operation.take();
let mut rows = std::mem::take(&mut self.encrypted_rows);
self.encrypted_rows_bytes = 0;
```

- **What:** Moves the held rows and their operation out of `self` and zeroes the byte counter.
- **Why here:** Done before any fallible step, so every exit leaves `self` with an empty buffer.
- **Assumes:** Nothing.
- **Establishes:** After L476 the Backend holds no rows regardless of outcome. A row is either returned in `Ok` or dropped.
- **Depended on by:** Caller L233 copies `encrypted_rows_operation` before the call because L474 clears it; it needs the value for `local_execution_failed` at L238-L239.

### Block 2: portal lookup and variant check

```rust
// L478-L488
let portal = operation
    .map(|operation| self.context.get_portal_from_execute(operation))
    .transpose()?
    .flatten();
let portal = match portal.as_deref() {
    Some(Portal::Encrypted { .. }) => portal.unwrap(),
    _ => { ...; return Err(ProtocolError::HeldDataRowsNotEncrypted.into()); }
};
```

- **What:** Resolves the `Arc<Portal>` for the operation and requires the `Encrypted` variant.
- **Why here:** Everything below (format codes, projection columns) is read from the portal. `Portal::format_codes` is `unreachable!()` for `Passthrough` (context/portal.rs L69-L71), so this match is what keeps L500 off that arm.
- **Assumes:** The portal now attached to the operation's `ExecuteContext` is the one that was attached when each row was held. The lookup is repeated here, not carried from `data_row_handler` (L735-L738).
- **Establishes:** `portal` is `Portal::Encrypted`. The `Arc` is a snapshot; `format_codes` and `statement.projection_columns` cannot change for the rest of the call.
- **Error paths:** `?` at L480 propagates `ProtocolStateUnavailable` (poisoned lock, context/mod.rs L1297), `UnknownOperation` (L1301), `OperationWithoutExecute` (L1305). `operation == None`, `execute.portal == None` (L1306), and `Portal::Passthrough` all reach L486. On each of these the rows taken at L475 are dropped.
- **Depended on by:** L500, L502.

### Block 3: width from the first row

```rust
// L491-L494
let result_column_count = match rows.first() {
    Some(row) => row.columns.len(),
    None => return Ok(Vec::new()),
};
```

- **What:** Takes the first row's column count as the width of every row in the batch.
- **Why here:** The width feeds `format_codes` (L500) and the chunking at L547.
- **Assumes:** Every row in `rows` has `columns.len() == result_column_count`. Nothing in this function or in the callers (L262-L301) compares row widths. The only source of uniformity is PostgreSQL's own framing of one result set.
- **Assumes:** `result_column_count > 0`. No check between L491 and L547.
- **Note on ordering:** The empty-buffer return at L493 sits after the portal check, so an empty buffer with no operation returns `Err` at L486, not `Ok` at L493. Callers do not reach this (L231, L286).

### Block 4: format codes and projection columns

```rust
// L500-L502
let result_column_format_codes = portal.format_codes(result_column_count);
let projection_columns = portal.projection_columns();
```

- **What:** Reads the two positional vectors that the rest of the function zips against row columns.
- `Portal::format_codes(row_len)` (context/portal.rs L56-L73): 0 stored codes gives `row_len` Text codes (L59); 1 stored code gives `row_len` copies (L60-L66); 2 or more gives a clone of the stored vector with no comparison to `row_len` (L67).
- Stored codes come from the client's Bind `result_formats` (rewrite/bind.rs L387-L391, frontend.rs L1050-L1054). The simple-query portal stores an empty vector (context/portal.rs L30-L37, frontend.rs L531).
- `Portal::projection_columns` returns `&statement.projection_columns` (context/portal.rs L46).
- **Assumes:** `projection_columns.len() == result_column_count` and position `i` in both refers to the same result column.
  What establishes it: `ColumnMapper::get_projection_columns` pushes one entry per `typed_statement.projection.columns()` (column_mapper.rs L38-L57). That is the mapper's view of the projection under the schema snapshot at Parse/Query time. Nothing compares it with the DataRow width or with the RowDescription field count; `row_description_handler` also zips without a length check (backend.rs L683).
- **Assumes (simple Query):** the portal's statement is the statement that produced these rows.
  `query_handler` assigns `portal` inside a loop over all statements of the Query and each `Route::Map` iteration overwrites it (frontend.rs L476, L480-L531). One portal is registered for the Query's operation (frontend.rs L568-L569, context/lifecycle.rs L377). So for a multi-statement Query the portal holds the last mapped statement's `projection_columns`, and it is set to `Encrypted` for any mapped statement whether or not that statement has a projection (frontend.rs L531 has no `has_projection` guard, in contrast to the Bind path at frontend.rs L1049).
  Nothing in `flush_encrypted_rows` or `data_row_handler` distinguishes which statement of the Query a DataRow belongs to.
- **Depended on by:** L507, L512, L555.

### Block 5: parse ciphertexts

```rust
// L505-L508
let ciphertexts: Vec<Option<EqlCiphertext>> = rows
    .iter_mut()
    .flat_map(|row| data_row::as_ciphertext(row, projection_columns))
    .collect::<Vec<_>>();
```

- **What:** Builds one flat vector over all rows.
- **Callee `as_ciphertext`** (rewrite/data_row.rs L16-L47), every path:
  - Iterates `row.columns.iter_mut().zip(column_configuration)` (L21). Output length per row is `min(row.columns.len(), projection_columns.len())`. Columns beyond `projection_columns.len()` produce no entry.
  - Config `None`: pushes `None` (L22-L23). The column bytes are not inspected.
  - Config `Some`, column NULL: pushes `None` (L24).
  - Config `Some`, column present, `to_eql_ciphertext` succeeds: pushes `Some(ciphertext)`.
  - Config `Some`, column present, `to_eql_ciphertext` fails: logs at `error!` (L34-L38), `.ok()` turns the failure into `None` (L41), and the column bytes stay as they were. No error reaches the caller. `as_ciphertext` returns `Vec`, not `Result` (L19).
  - The `EncryptError::ColumnIsNull` arm (L28-L32) is the only writer of `*data_column = None`. `to_eql_ciphertext` returns `ColumnCouldNotBeParsed` (L96, L102, L138) or a `serde_json::Error` converted through `From<serde_json::Error> for Error` (error.rs L648-L651) into the `#[from] serde_json::Error` variant at error.rs L320. No path in `to_eql_ciphertext` (L94-L113) constructs `ColumnIsNull`.
- **Callee `to_eql_ciphertext`** (rewrite/data_row.rs L94-L113): strips one leading `0x01` byte when present (L99-L100), otherwise parses all bytes as JSON (L101). The choice is by first byte and does not consult the portal's format code. A root-level `"s"` key reshapes the object into a one-entry SteVec document (L108-L110, L133-L151). Final deserialisation into `EqlCiphertext` is serde (L112); no identifier, version, or keyset check happens here.
- **Establishes:** `ciphertexts[k]` is `Some` only where the aligned projection entry is `Some` and the bytes deserialised.
- **Establishes:** `ciphertexts.len() == sum over rows of min(row.columns.len(), projection_columns.len())`.
- **Does not establish:** `ciphertexts.len() == rows.len() * result_column_count`. That equality holds only when every row has width `result_column_count` and `projection_columns.len() >= result_column_count`. L547 depends on it.

### Block 6: column configuration check

```rust
// L510-L512
let start = Instant::now();
self.check_column_config(projection_columns, &ciphertexts)?;
```

- **What:** Compares ciphertext identifiers with projection column identifiers.
- **Callee `check_column_config`** (backend.rs L569-L600): iterates `projection_columns.iter().zip(ciphertexts)` (L574). `ciphertexts` is the flat vector over all rows, so the zip ends after `projection_columns.len()` entries. With the alignment Block 5 produces, those entries are the first row's.
  - `(Some(col), Some(ct))`: error when `col.identifier != ct.identifier()` (L576-L584).
  - `(Some, None)`, `(None, None)`: accepted (L586, L588).
  - `(None, Some(ct))`: error (L590-L596). `as_ciphertext` never yields `Some` for a `None` config (data_row.rs L22-L25), so with aligned input this arm has no producer.
- **Establishes:** For the first `projection_columns.len()` ciphertext positions of the batch, each present ciphertext's `i` identifier equals the configured column's identifier.
- **Does not establish:** The same for positions beyond `projection_columns.len()`, which is every row after the first. A batch is at most 4096 rows or 64 MiB (backend.rs L75-L76, L287-L288), and each batch gets one covered row.
- **Why here:** Runs before the service call, so a mismatch in the covered positions returns `Err` without contacting ZeroKMS and without incrementing `DECRYPTION_ERROR_TOTAL` (the `inspect_err` is attached to `decrypt` only, L522-L524).
- **Timer:** `start` is taken at L510, so `duration` (L526) covers this check, the keyset read and the decrypt call, and excludes the JSON parsing of Block 5.

### Block 7: keyset read and decrypt

```rust
// L514-L524
let keyset_id = self.context.keyset_identifier()?;
debug!(target: CONTEXT, client_id = ..., ?keyset_id);
let plaintexts = self.context.decrypt(ciphertexts).await.inspect_err(|_| {
    counter!(DECRYPTION_ERROR_TOTAL).increment(1);
})?;
```

- **What:** Reads the keyset for logging, then decrypts the whole batch in one call.
- The `keyset_id` bound at L514 is used only by the `debug!` at L516-L519. `Context::decrypt` reads the keyset again under a separate lock acquisition (context/mod.rs L1681) and passes that value to the service (L1682).
- `keyset_identifier` returns `state.settings.keyset()` (context/mod.rs L1638-L1644), which is the transaction-scoped value when present, else the session value (context/settings.rs L280-L282, L389-L391).
- **Timing fact:** the keyset is the value in `ConnectionSettings` at the moment of the flush. It is not captured at Bind, at Execute, or when the rows were held. Neither `Portal` (context/portal.rs L6-L15) nor `ExecuteContext` (context/mod.rs L77-L83) has a keyset field.
- **Ordering fact in the caller:** on the non-DataRow path the flush at backend.rs L234 runs before `report_outcome` for the triggering message at L254, so a `CommandComplete` that triggers the flush has not yet been applied to settings when the keyset is read.
- **Callee `Context::decrypt`** (context/mod.rs L1677-L1683): reads keyset, delegates to `self.encryption.decrypt`. No check of the returned length. The sibling `Context::encrypt` does check (`encrypted.len() != expected`, L1667-L1673).
- **Callee `ZeroKms::decrypt` -> `decrypt_eql(.., SteVecAuthentication::RootOnly)`** (proxy/zerokms/zerokms.rs L368-L375, L388-L469), every path:
  - No default keyset and no connection keyset: `Err(MissingKeysetIdentifier)` (L396-L398). This precedes the empty-input check, so it applies to a batch with no ciphertexts too.
  - `init_cipher(keyset_id)` (L400, L155-L232): cache lookup keyed by the keyset's display string or `"default"` (L147-L152, L162); on miss, `ScopedCipher::init` with the connection keyset or the configured default (L178-L184). Errors: `UnknownKeysetIdentifier`, `AuthenticationFailed`, `Error::ZeroKMS` (L218-L229).
  - The check that each ciphertext's embedded keyset id equals the cipher's keyset id runs only for `AllEntries` (L401-L408). For `RootOnly`, the mode used here, no such comparison is made in the Proxy.
  - Scalar payload: one record (L423-L426). SteVec document: `ste_vec_entries_to_authenticate` returns the first entry only for `RootOnly` (L472-L483), errors with `SteVecMissingRootEntry` on an empty `sv` (L476-L478); `decode_ste_vec_selector` can error (L431).
  - No records: `Ok(vec![None; ciphertexts.len()])` (L444-L446).
  - `cipher.decrypt(records, &opts)` (L452-L455): black box (cipherstash-client). Error mapped through `ZeroKMSError::from`.
  - Result: `vec![None; ciphertexts.len()]` filled by `result_positions.into_iter().zip(decrypted)` (L461-L466). The zip truncates silently if `decrypted` is shorter than `records_to_decrypt`; the affected positions stay `None`.
  - `Plaintext::from_slice` failure maps to an `EncryptError` (L464).
- **Establishes (ZeroKms only):** `plaintexts.len() == ciphertexts.len()` and `plaintexts[k]` is `Some` only where `ciphertexts[k]` was `Some`.
- **Assumes:** A `None` plaintext at a position whose ciphertext was `Some` does not occur. With `ZeroKms` it occurs only through the L462 zip truncation.
- **Error effect:** any `Err` here returns from `flush_encrypted_rows`; the whole batch is dropped, not just the failing value.

### Block 8: timing and metrics

```rust
// L526-L544
let duration = Instant::now().duration_since(start);
if let Some(operation) = operation {
    self.context.add_decrypt_duration_for_execute(operation, duration)?;
}
if self.context.prometheus_enabled() { ... }
```

- **What:** Records the decrypt duration on the statement metrics scope and updates Prometheus counters.
- **Callee `add_decrypt_duration_for_execute`** (context/mod.rs L1935-L1947): calls `get_execute(operation)?` (L1941), which errors with `UnknownOperation` or `OperationWithoutExecute` (context/mod.rs L1314-L1321). A missing metrics scope is a no-op (L1870-L1873).
- **Assumes:** The operation still has an `ExecuteContext` after the `.await` at L522. It was present at L479; the two lookups are separate lock acquisitions with a suspension point between them.
- **Effect of the `?` at L531:** an error here returns `Err` after a successful decrypt, and the decrypted batch is dropped.
- `decrypted_count` counts `Some` plaintexts (L536-L539).

### Block 9: chunk, encode, rewrite

```rust
// L547-L566
let rows = plaintexts.chunks(result_column_count).zip(rows);
let mut messages = Vec::with_capacity(rows.len());
for (chunk, mut row) in rows {
    let data = chunk.iter().zip(result_column_format_codes.iter())
        .map(|(plaintext, format_code)| match plaintext {
            Some(plaintext) => to_sql(plaintext, format_code),
            None => Ok(None),
        })
        .collect::<Result<Vec<_>, _>>()?;
    data_row::rewrite(&mut row, &data)?;
    messages.push(BackendMessage::DataRow(row));
}
Ok(messages)
```

- **What:** Re-associates the flat plaintext vector with rows by fixed-width chunking, encodes each plaintext, overwrites row columns, and collects the rows.
- **Chunking (L547):** chunk `n` is `plaintexts[n*W .. (n+1)*W]` with `W = result_column_count`. Row `n` is paired with chunk `n`.
  - Row `n`'s plaintexts actually start at offset `sum of min(width_j, P)` over earlier rows `j`, with `P = projection_columns.len()` (Block 5). The two offsets agree only when every row contributes exactly `W` entries.
  - When `P < W`, each row contributes `P` entries. Chunk `n` then spans entries from more than one row, and there are `ceil(rows.len() * P / W)` chunks, fewer than `rows.len()`. The `zip` ends at the shorter side, so the trailing rows are consumed from the iterator and never pushed to `messages`.
  - When `P == 0` (an `Encrypted` portal whose statement has no projection, constructible on the simple-query path, frontend.rs L531), `plaintexts` is empty, there are zero chunks, the loop body never runs, and `Ok(vec![])` is returned for a non-empty batch.
  - The final chunk can be shorter than `W` (`chunks` yields a short tail).
  - `slice::chunks` has a documented panic when the chunk size is 0. `result_column_count` is 0 when the first row has no columns (L491-L492). No guard exists between L491 and L547. On that input the path to L547 requires only that L512 and L522 return `Ok`; with zero ciphertexts `check_column_config` iterates nothing and `ZeroKms::decrypt_eql` returns `Ok(vec![])` when a keyset is available (zerokms.rs L396-L400, L444-L446).
- **Format-code zip (L553-L555):** `data.len() == min(chunk.len(), result_column_format_codes.len())`. With 2 or more stored codes the vector length is the client's Bind count (context/portal.rs L67). Fewer codes than columns leaves the trailing columns without an entry in `data`; more codes than columns are ignored.
- **Callee `to_sql`** (postgresql/data/to_sql.rs L8-L15): always `Ok(Some(bytes))` or `Err`; never `Ok(None)`.
  - Text (L17-L34): `to_string()` of the inner value; any variant whose inner value is `None` produces the empty string (L30), which is `Some(empty)` and not SQL NULL.
  - Binary (L36-L58): the PostgreSQL type used for encoding is chosen by the `Plaintext` variant (L40-L49), not by the projection `Column`. `to_sql` receives no column. `Plaintext::BigUInt` is `unimplemented!()` (L51). The `IsNull` payload of `to_sql_checked` is discarded (L54-L55). Encoding failure maps to `PlaintextCouldNotBeEncoded` (L56).
- **Assumes:** The `Plaintext` variant produced by `Plaintext::from_slice` on decrypted bytes (zerokms.rs L464) matches the type OID that `row_description_handler` advertised for that position from `col.postgres_type` (backend.rs L674-L688). Nothing in this function compares the variant with the column's type.
- **Callee `data_row::rewrite`** (rewrite/data_row.rs L49-L65): `Err(DataRowColumnCountMismatch)` when `plaintexts.len() > row.columns.len()` (L50-L56). Otherwise, for each `Some` plaintext, overwrites the column at the same index only when that column is currently `Some` (L57-L62). A `None` plaintext leaves the column untouched. Columns at indexes `>= plaintexts.len()` are untouched.
- **Consequence of the `None` rule:** a column whose projection entry is `Some` and whose bytes failed to deserialise (Block 5), or whose plaintext came back `None`, is forwarded with its original stored bytes. No error is raised on that path in this function.
- **Width difference between rows:** a later row narrower than `W` makes `data.len() > row.columns.len()` possible, which is the one case `rewrite` rejects (L50). A later row wider than `W` passes `rewrite` and shifts every subsequent row's alignment.
- **Atomicity:** `messages` is local. A `?` at L560 or L562 on row `n` discards rows `0..n` already rewritten in this call. Rows returned by earlier flushes of the same execution (the L287-L295 size-triggered flush) have already been handed to pg-proto.
- **Establishes:** Each returned DataRow has the same column count as the held row it was built from (`rewrite` never changes `row.columns.len()`).

---

**Cross-Function Dependencies:**

Callees:

- `Context::get_portal_from_execute` (internal, context/mod.rs L1290-L1307): depended on to return the portal attached to this operation's `ExecuteContext`. Errors on unknown operation or missing execute; returns `Ok(None)` when the execute has no portal.
- `Portal::format_codes` (internal, context/portal.rs L56-L73): depended on to return one code per result column. It does so for 0 or 1 stored codes; for 2 or more it returns the stored vector unchanged.
- `Portal::projection_columns` (internal, context/portal.rs L43-L49): depended on to be positional over the DataRow's columns. It returns whatever `Statement.projection_columns` held at Parse/Query time.
- `data_row::as_ciphertext` (internal, rewrite/data_row.rs L16-L47): depended on to return one entry per row column. It returns `min(row width, projection length)` entries and converts parse failures to `None`.
- `Backend::check_column_config` (internal, backend.rs L569-L600): depended on to reject ciphertexts whose identifier differs from the configured column. It covers the first `projection_columns.len()` flat positions only.
- `Context::keyset_identifier` (internal, context/mod.rs L1638-L1644): current keyset from `ConnectionSettings`; value used for logging only at L514.
- `Context::decrypt` (internal, context/mod.rs L1677-L1683): reads the keyset again and delegates. Adds no length or content check.
- `EncryptionService::decrypt` (trait, proxy/mod.rs L164-L168; production impl `ZeroKms::decrypt` zerokms.rs L368-L375): depended on for output length equal to input length and positional correspondence. The trait does not state this; `ZeroKms::decrypt_eql` provides it (L445, L461).
- `ScopedCipher::init`, `cipher.decrypt`, `Plaintext::from_slice` (external-black-box, cipherstash-client): sent the keyset identifier and the ciphertext records. Assumed to fail for records not decryptable under the cipher's keyset, to return one output per record in order, and to authenticate the record. None of this is checked in the Proxy on the `RootOnly` path.
- `Context::add_decrypt_duration_for_execute` (internal, context/mod.rs L1935-L1947): can fail after a successful decrypt.
- `to_sql` (internal, postgresql/data/to_sql.rs L8-L58): encodes by `Plaintext` variant and format code; has two panic sites reachable by variant (`unimplemented!()` L51) and none by format code.
- `data_row::rewrite` (internal, rewrite/data_row.rs L49-L65): overwrites non-NULL columns positionally; rejects only `plaintexts.len() > row.columns.len()`.

Callers:

- `Backend::intercept_backend`, backend.rs L234: on any non-DataRow message when rows are held. `Ok` rows become the prefix of the triggering message (L371-L376). `Err` sets `discard_execution` (L237), calls `local_execution_failed` (L238-L239), reports the triggering message's outcome as discarded when correlated (L244-L246), and returns a single ErrorResponse through `decryption_failure` (L247, L122-L128).
- `Backend::intercept_backend`, backend.rs L294: when the buffer reaches 4096 rows or 64 MiB (L287-L288). `Ok` becomes `Expand(messages)` (L295). `Err` calls `local_execution_failed(Some(operation))` and `decryption_failure` (L297-L298).
- Both callers treat `Ok(messages)` as the complete replacement for the held rows. Neither compares `messages.len()` with the number of rows that were held.

Shared state:

- `Backend.encrypted_rows`, `encrypted_rows_operation`, `encrypted_rows_bytes`: written here (L474-L476), in `intercept_backend` (L271-L286), in `decryption_failure` (L123-L125).
- `ConnectionProtocolState.operations[..].execute.portal`: read here (L479) and in `data_row_handler` (L736); written on the frontend path (`finish_bind`, `register_query`) and removed by backend lifecycle functions (context/mod.rs L371, L541, L545).
- `ConnectionProtocolState.settings.keyset`: read here twice (L514, context/mod.rs L1681); written by the Backend on CommandComplete and ReadyForQuery per the system context.
- `ConnectionProtocolState.statement_metrics`: written at L530-L531.
- `ZeroKms.cipher_cache`: process-wide, read and populated through L522.

Invariant couplings:

- `row_description_handler` (backend.rs L674-L688) rewrites type OIDs from the same `statement.projection_columns` with the same unchecked positional zip. The DataRow rewrite here and the RowDescription rewrite there share the assumption that `projection_columns` is positional over PostgreSQL's result columns, and neither validates the other.
- The portal variant decides buffering (`data_row_handler` L739-L751) and the same variant is required here (L482-L488); the row content is not consulted by either.
- The keyset used to decrypt is decoupled from the keyset in effect when the statement was parsed, bound, or executed; coupling exists only through `ConnectionSettings` at flush time.

---

**Open Questions:**

- unclear; need to inspect `set_simple_query_execute_until_ready` (context/mod.rs L657) and `install_portal` to confirm that, for a simple Query, `ExecuteContext.portal` is the single portal registered at context/lifecycle.rs L377 for every result set of that Query.
- unclear; need to inspect `Frontend::route` and `Route::Map` construction to determine which statements of a multi-statement simple Query take the `Route::Map` arm at frontend.rs L482-L535, and so which statement's `projection_columns` the Query's portal ends up holding.
- unclear; need to inspect eql-mapper's `projection.columns()` (wildcard expansion, `RETURNING`, set operations) to determine when its length differs from the column count PostgreSQL returns, including when the table changed after the schema snapshot used at Parse.
- unclear; need to inspect cipherstash-client `ScopedCipher::decrypt` to determine whether a record is bound to the `i` identifier, to the keyset, and to the column descriptor, since on the `RootOnly` path the Proxy compares the identifier for the first row of a batch only and never compares the keyset.
- unclear; need to inspect cipherstash-client `ScopedCipher::decrypt` to confirm the returned vector has one element per input record in order; `decrypt_eql` L462 truncates silently otherwise.
- unclear; need to inspect `Plaintext::from_slice` and `postgres_types` `ToSql for Option<T>` to determine what `binary_to_sql` writes for a variant whose inner value is `None` (to_sql.rs L40-L49, L54-L55).
- unclear; need to inspect pg-proto's handling of `Err` returned from `backend_operation` (postgresql/middleware/mod.rs L65-L76) for the case where `local_execution_failed` itself fails with `?` at backend.rs L238-L239 after `discard_execution` was set at L237.
- unclear; need to inspect pg-proto's task model to determine whether Frontend handlers can run during the `.await` at backend.rs L522, which bears on the two-lookup assumption at L479 and L530-L531.
- unclear; need to inspect pg-proto's `DataRow` decoding to determine whether a DataRow with a column count of 0 is delivered to the middleware as `columns: vec![]`.
