## `ZeroKms::decrypt_eql` in packages/cipherstash-proxy/src/proxy/zerokms/zerokms.rs (L388-L469)

Paths are relative to `packages/cipherstash-proxy/src/` unless they start with `packages/`.
`zerokms.rs` means `proxy/zerokms/zerokms.rs`.
Bare line numbers (`L400`) refer to `zerokms.rs`.

Signature (L388-L393):

```rust
async fn decrypt_eql(
    &self,
    keyset_id: Option<KeysetIdentifier>,
    ciphertexts: Vec<Option<EqlCiphertextV3>>,
    ste_vec_authentication: SteVecAuthentication,
) -> Result<Vec<Option<Plaintext>>, Error>
```

**Purpose:** Turns a positional vector of optional EQL v3 payloads into a positional vector of optional plaintexts, using one `ScopedCipher` chosen by the caller-supplied keyset.
It is the single implementation behind both `EncryptionService::decrypt` (database reads, `RootOnly`, L373) and `EncryptionService::decrypt_inbound_eql` (application-supplied storage payloads, `AllEntries`, L382).
The function is private to the `impl ZeroKms` block at L387; those two trait methods are its only callers.

Source availability for callees: `cipherstash-client` is pinned at `=0.42.2` (Cargo.toml L50, Cargo.lock L752-L753) and its source is not on disk (the local cargo registry holds only `cipherstash-config-0.42.3` and `zerokms-protocol-0.12.31`, neither of which is the locked version or defines the types used here).
`ScopedCipher::init`, `ScopedCipher::keyset_id`, `ScopedCipher::decrypt`, `KeyHeader::record_with_selector`, the `Decryptable` impls of `EncryptedRecord` and `RecordWithNonce`, `DecryptOptions::default`, and `Plaintext::from_slice` are therefore black boxes in this analysis.

---

**Inputs & Assumptions:**

- `keyset_id: Option<KeysetIdentifier>`.
  `KeysetIdentifier` wraps cipherstash-client's `IdentifiedBy` (postgresql/context/mod.rs L48-L49).
  Both production callers read it from `ConnectionProtocolState.settings` at call time: `Context::decrypt` (postgresql/context/mod.rs L1681) and `Context::decrypt_inbound_eql` (postgresql/context/mod.rs L1689), each through `keyset_identifier` (postgresql/context/mod.rs L1638-L1644).
  It is client-selectable through `SET CIPHERSTASH.KEYSET_*` when no default keyset is configured; `validate_setting` refuses a keyset setting when a default is configured (postgresql/context/lifecycle.rs L521-L528).
- `ciphertexts: Vec<Option<EqlCiphertextV3>>`, owned.
  Content trust differs by mode.
  `RootOnly`: values deserialised from stored column bytes by `to_eql_ciphertext` (postgresql/rewrite/data_row.rs L94-L113), written by any party able to write the rows.
  `AllEntries`: values deserialised from a Bind parameter of the authenticated client and passed through `validate_storage_metadata` (postgresql/inbound_eql.rs L61-L64, L94-L125).
- `ste_vec_authentication: SteVecAuthentication` (L54-L58), a compile-time constant at each call site (L373, L382).
  It selects two things: whether the keyset comparison at L401-L408 runs, and how many entries of each SteVec document are submitted for decryption (L472-L483).
- `self.default_keyset_id: Option<Uuid>` (L115), operator configuration copied at `ZeroKms::init` (L137).
- `self.cipher_cache` (L117), process-wide, reached through `init_cipher`.
- The function takes no `Column`, no identifier, and no expected type.
  Nothing in L388-L469 relates a payload to a table, a column, or a plaintext type.

Preconditions, and what establishes each:

| Precondition | Established by |
|---|---|
| A keyset is available (argument or configured default) | L396-L398, in this function. |
| The cipher is scoped to the keyset the caller named | `init_cipher` L159-L184 plus black-box `ScopedCipher::init`. Cache hit path (L162-L166) returns whatever is stored under the key string. |
| Each payload's embedded keyset id equals the cipher's keyset id | `AllEntries`: L401-L408. `RootOnly`: nothing found in the Proxy; rests on black-box `ScopedCipher::decrypt`. |
| Each payload belongs to the column it was read from or is destined for | Not in this function. `AllEntries`: `validate_storage_metadata` (postgresql/inbound_eql.rs L98-L114). `RootOnly`: `Backend::check_column_config`, which dossier 02 records as covering the first row of a batch only (postgresql/middleware/backend.rs L569-L600). |
| `sv[0]` of a SteVec document is the entry whose plaintext is the value for the column | nothing found in this function; position 0 is taken as the root (L437, L476-L478). |
| `cipher.decrypt` returns exactly one output per submitted record, in submission order | nothing found. L462 zips without a length comparison. |
| `cipher.decrypt` returns `Err` for the whole call when any single record fails authentication | nothing found (black box). |
| Decrypted root bytes decode to the `Plaintext` variant the column expects | nothing found. `Plaintext::from_slice` (L464) receives bytes only. |

---

**Outputs & Effects:**

- `Ok(Vec<Option<Plaintext>>)` with `len() == ciphertexts.len()` on both success exits (L445, L461, L468).
- `result[idx]` is `Some` only where `ciphertexts[idx]` was `Some` (positions come only from L425 and L437).
- The converse (`Some` in gives `Some` out) holds only when `decrypted.len() >= result_positions.len()`; see Block 6.
- `Err(Error)` on: missing keyset (L397), any `init_cipher` error (L400), keyset mismatch in `AllEntries` (L407), empty `sv` (L429 via L476-L478), malformed selector (L431), `cipher.decrypt` error (L455), `Plaintext::from_slice` error (L464).
  Every error is all-or-nothing for the whole input vector; no partial result is returned.
- State: no field of `self` is written directly.
  `init_cipher` inserts into the process-wide `cipher_cache` on a miss (L200-L205).
- External interaction: `ScopedCipher::init` on cache miss (L184) and one `ScopedCipher::decrypt` call (L452-L454).
- Suspension points: L400 and L454.
  `ciphertexts`, `result_positions` and `records_to_decrypt` are locals; the keyset was already read by the caller before entry, so a settings change during either `.await` does not alter which cipher this call uses.
- Logging: `debug!` at L394 (keyset ids), L450 (record count), L457 (output count and duration).
  No ciphertext or plaintext content is logged in L388-L469.

---

**Block-by-Block:**

### Block 1: keyset presence

```rust
// L396-L398
if self.default_keyset_id.is_none() && keyset_id.is_none() {
    return Err(EncryptError::MissingKeysetIdentifier.into());
}
```

- **What:** Refuses when neither an argument keyset nor a configured default exists.
- **Why here:** Precedes `init_cipher`, so `init_cipher(None)` is reached from this function only when `default_keyset_id` is `Some`.
- **Ordering fact:** This precedes the empty-input return at L444-L446, so a vector of all `None` still fails here without a keyset.
- **Does not establish:** That an argument keyset is absent when a default is configured.
  When both are `Some`, `init_cipher` gives the argument precedence (L178-L181).
  The exclusion of that combination is `validate_setting` on the frontend path (postgresql/context/lifecycle.rs L521-L528), not this function.
- **Mirror:** `encrypt` has the same check at L249-L251.

### Block 2: cipher acquisition

```rust
// L400
let cipher = self.init_cipher(keyset_id).await?;
```

- **What:** Obtains an `Arc<ScopedCipher>`; `keyset_id` is moved and not used again in this function.
- **Callee `init_cipher`** (L155-L232), every path:
  - Cache key: `format!("{}", id.0)` for `Some`, the literal `"default"` for `None` (L147-L152).
    The key is the `Display` rendering of `IdentifiedBy`; UUID and name identifiers share one string namespace with the literal `"default"`.
  - Hit: returns the cached `Arc` (L162-L166) with no call to ZeroKMS.
    Entries live until TTL or capacity eviction (L130-L131).
  - Miss: `identified_by` is the argument, else `default_keyset_id` as `IdentifiedBy::Uuid`, else `None` (L178-L181); then black-box `ScopedCipher::init(zerokms_client, identified_by)` (L184).
  - Miss, `Ok`: insert under the cache key and return (L200-L213).
    The lookup at L162 and the insert at L200-L202 are separate operations with an `.await` between them (L184); concurrent misses for one key each call `ScopedCipher::init`.
  - Miss, `Err`: `LoadKeyset` maps to `UnknownKeysetIdentifier` (L219-L224), `Auth` to `AuthenticationFailed` (L225-L227), anything else to `Error::ZeroKMS` (L228).
    Nothing is cached on error.
- **Other writer of the `"default"` entry:** `Proxy::init` calls `init_cipher(None)` unconditionally (proxy/mod.rs L75).
  When no default keyset is configured that call passes `identified_by = None` to `ScopedCipher::init` (L178-L181) and caches the result under `"default"`.
  From `decrypt_eql`, the `"default"` key is then reachable only through an argument keyset whose `Display` string is `"default"`, because L396 rejects `None` in that configuration.
- **Establishes:** `cipher` is the cache's (or a freshly initialised) cipher for the key string derived from the caller's keyset.
- **Assumes:** `ScopedCipher::init` fails for a keyset the Proxy's credentials are not granted, and the cached cipher stays valid for the TTL.
  Both are black-box.
- **Depended on by:** L405 (`cipher.keyset_id()`), L452 (`cipher.decrypt`).

### Block 3: keyset comparison, inbound mode only

```rust
// L401-L408
if matches!(ste_vec_authentication, SteVecAuthentication::AllEntries)
    && ciphertexts.iter().flatten()
        .any(|ciphertext| ciphertext_keyset_id(ciphertext) != Some(cipher.keyset_id()))
{
    return Err(EncryptError::InvalidInboundEqlPayload.into());
}
```

- **What:** In `AllEntries` mode, rejects the whole input when any present payload's embedded keyset id differs from the cipher's.
- **Callee `ciphertext_keyset_id`** (L485-L490): scalar reads `payload.ciphertext.keyset_id`; SteVec reads `document.key_header.keyset_id`.
  Both are `Option<Uuid>`; a payload with no embedded keyset id compares unequal to `Some(..)` and is rejected.
- **Callee `cipher.keyset_id()`:** black box; the comparison type-checks against `Option<Uuid>`, so it returns a `Uuid`.
- **Granularity:** A SteVec document carries one keyset id in its `h` header (L488).
  Individual `sv` entries have no keyset field in the Proxy's view (`SteVecEntryV3` construction at L521-L526 lists `selector`, `ciphertext`, `is_array`, `term`), and every entry is rebuilt against the same header at L433-L435.
- **Why here:** Runs before any record is built or sent, so a mismatched inbound payload never reaches `cipher.decrypt`.
- **`RootOnly` mode:** The first operand of `&&` is false and the comparison does not run.
  On the read path no line of the Proxy compares a stored payload's keyset id with the connection keyset.
  The `Decryptable` impl forwards `keyset_id()` from the wrapped record (L63-L68), which makes the value available to `ScopedCipher::decrypt`; what the library does with it is not visible.
- **Establishes (`AllEntries` only):** every present payload's header keyset id equals `cipher.keyset_id()`.
- **Does not establish:** any relationship between `cipher.keyset_id()` and the identifier the client set.
  That link is `init_cipher` and `ScopedCipher::init`.
- **Comment at L410-L412:** states that `decryption_policy` needs no structural check because ZeroKMS verifies a policy MAC during key retrieval for tag-version 1 records.
  This is a claim about the black box; no line in the Proxy checks `decryption_policy` or a record's tag version.

### Block 4: record construction and position bookkeeping

```rust
// L418-L442
let mut result_positions: Vec<Option<usize>> = Vec::new();
let mut records_to_decrypt: Vec<V3Record> = Vec::new();
for (idx, ciphertext) in ciphertexts.iter().enumerate() {
    match ciphertext {
        Some(EqlCiphertextV3::Encrypted(payload)) => {
            records_to_decrypt.push(V3Record::Scalar(payload.ciphertext.clone()));
            result_positions.push(Some(idx));
        }
        Some(EqlCiphertextV3::SteVec(document)) => {
            let entries = ste_vec_entries_to_authenticate(&document.ste_vec, ste_vec_authentication)?;
            for (entry_index, entry) in entries.iter().enumerate() {
                let selector = decode_ste_vec_selector(&entry.selector)?;
                records_to_decrypt.push(V3Record::SteVecEntry(
                    document.key_header.record_with_selector(entry.ciphertext.clone(), selector)));
                result_positions.push((entry_index == 0).then_some(idx));
            }
        }
        None => {}
    }
}
```

- **What:** Flattens payloads into decryptable records and records, per record, which output slot (if any) its plaintext fills.
- **Scalar arm (L423-L426):** one record, one `Some(idx)`.
  `V3Record::Scalar` reports no nonce override and no AAD selector (L84-L96).
  Only `payload.ciphertext` is used; the payload's `i` identifier and its SEM terms (`hm`, `bf`, `ob`, `op`) are not read here.
- **SteVec arm (L427-L439):**
  - **Callee `ste_vec_entries_to_authenticate`** (L472-L483): `entries.first()` is taken before the mode match, so an empty `sv` is `Err(SteVecMissingRootEntry)` in both modes (L476-L478).
    `RootOnly` returns a one-element slice of the first entry (L480); `AllEntries` returns the whole slice (L481).
    The returned slice is therefore never empty, so `entry_index == 0` occurs exactly once per document.
  - **Callee `decode_ste_vec_selector`** (L101-L111): `hex::decode` then conversion to `[u8; 16]`; any decode failure or a length other than 16 bytes is `Err(SteVecSelectorInvalid)` carrying the selector string (L102-L110).
    `hex` 0.4.3 accepts both `A-F` and `a-f` (hex-0.4.3/src/lib.rs L177-L178), so two different selector strings decode to the same 16 bytes.
    In `RootOnly` only the first entry's selector is decoded; the other entries of a stored document are not inspected at all.
  - **Callee `KeyHeader::record_with_selector`** (black box, L433-L435): sent the entry ciphertext bytes and the decoded selector; returns a `RecordWithNonce`.
    The enum doc at L38-L43 and L49-L51 states the nonce and AAD both derive from the selector; `V3Record::SteVecEntry` forwards `nonce_override()` and `aad_selector()` from it (L87, L94).
    The call is infallible at this site (no `?`).
  - **Position rule (L437):** `Some(idx)` for the entry at position 0 of the slice, `None` for every other entry.
    "Root" is defined by position in `sv`, not by selector value.
    No line here compares `sv[0].selector` with an expected root selector.
- **`None` arm (L440):** contributes nothing.
- **Establishes:**
  - `result_positions.len() == records_to_decrypt.len()` (each push at L424 / L432 is paired with one at L425 / L437).
  - Each `idx` with `Some` input appears exactly once as `Some(idx)` in `result_positions`; no `idx` with `None` input appears.
  - Every `Some` input contributes at least one record, or the function has already returned `Err`.
  - Record order is input order, and within a document, `sv` order.
- **Error scope:** `?` at L429 and L431 abandons the whole input.
  On the read path the input is every encrypted column of up to 4096 rows (dossier 02, postgresql/middleware/backend.rs L505-L508, L287-L288), so one stored value with an empty `sv` or a malformed first selector fails the batch.
- **Bare-entry reshaping upstream (read path):** `to_eql_ciphertext` turns any stored JSON object with a root-level `"s"` key into a one-entry SteVec document (postgresql/rewrite/data_row.rs L108-L110, L121-L123, L133-L151).
  For such a value position 0 is the extracted entry, not a document root, and its decrypted bytes go to `Plaintext::from_slice` at L464 like any root.
  The discriminator is the presence of `"s"` in the stored bytes; it does not depend on the projection column's `eql_term`.

### Block 5: empty short-circuit and decrypt call

```rust
// L444-L455
if records_to_decrypt.is_empty() {
    return Ok(vec![None; ciphertexts.len()]);
}
let opts = DecryptOptions::default();
let decrypted = cipher.decrypt(records_to_decrypt, &opts).await.map_err(ZeroKMSError::from)?;
```

- **What:** Returns all-`None` when there is nothing to decrypt; otherwise decrypts every record in one call.
- **Empty case:** By Block 4 this is reached only when every input is `None`.
  It is reached after `init_cipher`, so an all-`None` input still pays the cache lookup and, on a miss, a ZeroKMS round trip.
- **`DecryptOptions::default()` (L449):** no per-call keyset or context is supplied; the comment at L448 states the cipher's scope is relied on instead.
  What the default options contain is black-box.
- **Callee `ScopedCipher::decrypt`** (black box): sent `Vec<V3Record>` and default options.
  The `Decryptable` impl is the Proxy's whole contract with it: `keyset_id`, `retrieve_key_payload`, `into_encrypted_record` forwarded from the wrapped record (L63-L82); `nonce_override` and `aad_selector` forced to `None` for scalars and forwarded for entries (L84-L96).
  `type Error = Infallible` (L61).
- **Assumed about it, none checked here:**
  - returns `Err` when any record fails AEAD authentication, rather than omitting or substituting an element;
  - returns exactly `records_to_decrypt.len()` outputs in input order;
  - refuses a record whose keyset id differs from the cipher's scope (the only keyset enforcement on the `RootOnly` path);
  - binds an entry record to its selector through nonce and AAD, so an entry ciphertext presented under a different selector fails.
- **Error mapping:** `ZeroKMSError::from` is the `#[from] cipherstash_client::zerokms::Error` variant `System` (error.rs L107), then `?` converts to `Error`.
  The caller on the inbound path replaces every error from this function with `InvalidInboundEqlPayload` (postgresql/middleware/frontend.rs L1189-L1197), including `MissingKeysetIdentifier` and ZeroKMS availability or authentication errors.
- **Timing:** `decrypt_start` (L451) to L456 covers only the `cipher.decrypt` call.

### Block 6: placement

```rust
// L461-L468
let mut result: Vec<Option<Plaintext>> = vec![None; ciphertexts.len()];
for (result_position, bytes) in result_positions.into_iter().zip(decrypted) {
    if let Some(idx) = result_position {
        result[idx] = Some(Plaintext::from_slice(&bytes).map_err(EncryptError::from)?);
    }
}
Ok(result)
```

- **What:** Pairs each decrypted output with its bookkeeping slot by position and decodes the outputs that have a slot.
- **Indexing:** `result[idx]` cannot go out of bounds: every `idx` came from `ciphertexts.iter().enumerate()` (L421) and `result.len() == ciphertexts.len()` (L461).
- **No count comparison:** Nothing between L455 and L462 compares `decrypted.len()` with `result_positions.len()`.
  `Iterator::zip` stops at the shorter side.
  - `decrypted` shorter: the trailing slots are never visited and stay `None`, and the function returns `Ok`.
  - `decrypted` longer: the surplus outputs are dropped unread.
  - Same length but different order or a missing middle element: every later output lands in the slot of a different input.
  The sibling `encrypt` path has the comparison this path lacks: `place_ciphertexts` returns `EncryptionResultCountMismatch` when the counts differ (L502-L508), and `Context::encrypt` checks again (postgresql/context/mod.rs L1667-L1673).
  `Context::decrypt` and `Context::decrypt_inbound_eql` add no check (postgresql/context/mod.rs L1677-L1693).
- **What a `None` in a `Some` slot becomes downstream:**
  - Read path: `flush_encrypted_rows` encodes `None` as "leave the column untouched", and `data_row::rewrite` writes only `Some` values (postgresql/rewrite/data_row.rs L57-L62), so the stored bytes of that column are forwarded as they are (dossier 02, Block 9).
  - Inbound path: the plaintexts go to `Context::encrypt` (postgresql/middleware/frontend.rs L1207-L1209); `ZeroKms::encrypt` skips `None` plaintexts (L260) and leaves that output `None`; `merge_inbound_eql` requires `Some(EqlOutput::Store(_))` and returns `InvalidInboundEqlPayload` otherwise (postgresql/middleware/frontend.rs L1222-L1228).
- **Non-root outputs (L463):** when `result_position` is `None` the bytes are dropped without being read.
  In `AllEntries` mode the only use of a non-root entry's decryption is that `cipher.decrypt` did not return `Err`.
  The comment at L459-L460 states non-root plaintexts are sentinels that are not legal `Plaintext` values; no line checks what they are.
- **Callee `Plaintext::from_slice`** (black box): sent the decrypted bytes of each slotted record.
  Its error converts through `EncryptError::from` (L464); the `#[from]` candidates are `PlaintextCouldNotBeDecoded(TypeParseError)` (error.rs L420) and `Pipeline(EncryptionError)` (error.rs L417).
  A decode failure on one value fails the whole call, after ZeroKMS has already returned every plaintext.
- **Type of the result:** the `Plaintext` variant is whatever `from_slice` decodes from the bytes.
  No expected type is available to compare with (the function has no `Column`).
  On the read path `to_sql` then encodes by variant (dossier 02, Block 9); on the inbound path the plaintext is re-encrypted for the destination column and its terms compared (postgresql/middleware/frontend.rs L1207-L1237).
- **Establishes:** `result.len() == ciphertexts.len()`; `Some` only at slots whose input was `Some`.

---

**What each mode covers, stated from the code:**

| Property | `RootOnly` (L373) | `AllEntries` (L382) |
|---|---|---|
| Keyset presence | L396 | L396 |
| Payload keyset id equals cipher keyset id, checked in Proxy | not checked | L401-L408 |
| Scalar `c` submitted to `cipher.decrypt` | yes (L424) | yes (L424) |
| SteVec `sv[0]` submitted | yes (L480) | yes (L481) |
| SteVec `sv[1..]` submitted | no (L480) | yes (L481) |
| SteVec `sv[1..]` selectors decoded or inspected | no | yes (L431) |
| Envelope fields outside the record (`i`, `v`, SEM terms, entry `term` and `is_array`) | not read | not read here; compared by `sem_terms_match` in the caller (postgresql/inbound_eql.rs L195-L221, postgresql/middleware/frontend.rs L1230) |
| Descriptor / destination column | not read | not read here; `validate_storage_metadata` before the call (postgresql/inbound_eql.rs L104-L114) |
| Duplicate selectors within `sv` | not applicable | not checked here; `sem_terms_match` compares length and selectors pairwise in order against the re-derived document (postgresql/inbound_eql.rs L208-L217) |

---

**Cross-Function Dependencies:**

Callees:

- `ZeroKms::init_cipher` (internal, L155-L232): depended on to return a cipher scoped to the caller's keyset, or the configured default.
  Establishes the scope only on a miss, through black-box `ScopedCipher::init`; on a hit it returns the entry stored under the key string.
- `ScopedCipher::keyset_id` (external-black-box, L405): depended on to report the keyset the cipher decrypts under.
- `ciphertext_keyset_id` (internal, L485-L490): reads the header keyset id; one id per document.
- `ste_vec_entries_to_authenticate` (internal, L472-L483): depended on for a non-empty slice whose element 0 is `sv[0]`; establishes both on every path.
- `decode_ste_vec_selector` (internal, L101-L111): depended on for exactly 16 bytes; establishes it on every `Ok` path.
- `hex::decode` (external-source-available, hex-0.4.3/src/lib.rs): case-insensitive hex, error on odd length or non-hex.
- `KeyHeader::record_with_selector` (external-black-box, L433-L435): depended on to produce a record whose nonce and AAD derive from the supplied selector.
- `ScopedCipher::decrypt` (external-black-box, L452-L454): depended on for authentication of every record, keyset enforcement, one output per record, and input order.
- `DecryptOptions::default` (external-black-box, L449).
- `Plaintext::from_slice` (external-black-box, L464): depended on to reject bytes that are not an encoded plaintext.

Callers:

- `ZeroKms::decrypt` (L368-L375), `RootOnly`; reached from `Context::decrypt` (postgresql/context/mod.rs L1677-L1683), whose only caller is `Backend::flush_encrypted_rows` (postgresql/middleware/backend.rs L522).
  That caller depends on output length equal to input length and positional correspondence (dossier 02, Block 7 and Block 9).
- `ZeroKms::decrypt_inbound_eql` (L377-L384), `AllEntries`; reached from `Context::decrypt_inbound_eql` (postgresql/context/mod.rs L1685-L1693), whose only caller is `Frontend::merge_inbound_eql` (postgresql/middleware/frontend.rs L1185-L1197).
  That caller passes only `Some` values (postgresql/middleware/frontend.rs L1181-L1184) and depends on a `Some` plaintext per position for the re-encrypt-and-compare step.

Shared state:

- `ZeroKms.cipher_cache` (L117): read and possibly written through L400; also written by `Proxy::init` (proxy/mod.rs L75) and by `encrypt` (L253).
- `ZeroKms.default_keyset_id` (L115): read at L394 and L396; set once at L137.
- `ConnectionProtocolState.settings.keyset`: not read here; it is the source of the `keyset_id` argument in both callers.

Invariant couplings:

- `encrypt` and `decrypt_eql` select the cipher the same way (L249-L253, L396-L400), so a value this connection wrote and reads back under an unchanged keyset uses one cache entry.
- The inbound guarantee is split across three places that must all run: `validate_storage_metadata` (identifier, descriptor, term presence), `decrypt_eql` in `AllEntries` (keyset id, every record decrypts), and `sem_terms_match` (terms and selectors equal a fresh encryption).
  `decrypt_eql` alone covers only the middle one.
- The read-path guarantee is split between `check_column_config` (identifier, first row of a batch) and `decrypt_eql` in `RootOnly` (root record decrypts); keyset binding on that path is delegated entirely to the library.
- Positional placement here (L462) and positional chunking in `flush_encrypted_rows` (postgresql/middleware/backend.rs L547) both assume lengths that neither side compares.

---

**Open Questions:**

- unclear; need to inspect cipherstash-client 0.42.2 `ScopedCipher::decrypt` to determine whether the returned vector always has one element per input record in input order, and whether a single failing record makes the whole call return `Err`.
- unclear; need to inspect cipherstash-client 0.42.2 `ScopedCipher::decrypt` and the `Decryptable::keyset_id` consumer to determine whether a record whose keyset id differs from the cipher's scope is refused, since L401-L408 does not run for `RootOnly`.
- unclear; need to inspect cipherstash-client 0.42.2 `KeyHeader::record_with_selector` and `RecordWithNonce::{nonce_override, aad_selector}` to confirm that the selector is bound into both nonce and AAD, and whether the header's `descriptor` and `keyset_id` are covered by the entry's AEAD tag.
- unclear; need to inspect cipherstash-client 0.42.2 `EncryptedRecord` to confirm that `descriptor` and `keyset_id` are authenticated fields of a scalar record, as the comment at postgresql/inbound_eql.rs L104-L106 states.
- unclear; need to inspect cipherstash-client 0.42.2 `Plaintext::from_slice` to determine which byte strings it accepts, in particular whether the decrypted bytes of a non-root SteVec entry are rejected (comment at L459-L460) and what it returns for an extracted `eql_v3_json_entry` reshaped at postgresql/rewrite/data_row.rs L133-L151.
- unclear; need to inspect cipherstash-client 0.42.2 `IdentifiedBy`'s `Display` impl to determine whether a name identifier renders as the bare name, which decides whether a keyset named `default` (or named with a UUID string) shares a `cipher_cache` key with another identifier (L147-L152, proxy/mod.rs L75).
- unclear; need to inspect cipherstash-client 0.42.2 `ScopedCipher::init` with `identified_by = None` to determine which keyset the `"default"` cache entry created at proxy/mod.rs L75 is scoped to when no default keyset is configured.
- unclear; need to inspect cipherstash-client 0.42.2 `ScopedCipher` to determine whether a cached cipher re-checks access with ZeroKMS on each `decrypt` or holds its grant for the cache TTL (L131).
- unclear; need to inspect the ZeroKMS key-retrieval path to confirm the `decryption_policy` MAC claim in the comment at L410-L412 and which tag versions it applies to.
- unclear; need to inspect every frontend path that admits `SET CIPHERSTASH.KEYSET_*` to confirm `validate_setting` (postgresql/context/lifecycle.rs L521-L528) runs on each, since it is the only thing keeping `keyset_id` `None` when a default keyset is configured.
- unclear; need to inspect `Backend::check_column_config` and `as_ciphertext` alignment across rows (postgresql/middleware/backend.rs L569-L600) to confirm the first-row-only coverage recorded in dossier 02, since `decrypt_eql` itself performs no identifier check.
