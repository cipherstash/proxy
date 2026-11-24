# sem_terms_match

- File: `packages/cipherstash-proxy/src/postgresql/inbound_eql.rs`
- Lines: L195-L221 (private helpers `ste_vec_terms_match` L223-L231 and `bloom_filters_match` L233-L248)
- Signature: `pub fn sem_terms_match(inbound: &EqlCiphertext, derived: EqlCiphertext) -> bool`
- Sole production caller: `Frontend::merge_inbound_eql` at `postgresql/middleware/frontend.rs` L1230.
- Unit-test callers: inbound_eql.rs L435, L449, L461, L469.

Paths below are relative to `packages/cipherstash-proxy/src/` unless they start with `packages/`.
`IE` means `postgresql/inbound_eql.rs`, `FE` means `postgresql/middleware/frontend.rs`, `ZK` means `proxy/zerokms/zerokms.rs`, `CTX` means `postgresql/context/mod.rs`.

The format reference files named in the task (`ANALYSIS_FORMAT.md` and siblings) were not found on disk, so this document follows the layout of the earlier files in `audit-context/functions/`.

Source for `cipherstash-client` 0.42.2 (pinned at `Cargo.toml` L50) was not found under `~/.cargo/registry/src`.
Every type and function from that crate is therefore treated as a black box here.
Field lists for its structs are taken from exhaustive struct literals inside this repository, which the compiler accepts only when every field is named.

## 1. Purpose

A pure predicate over two EQL v3 storage payloads.
`inbound` is the payload the client supplied.
`derived` is the payload Proxy produced by decrypting `inbound` and re-encrypting the resulting plaintext for the destination column (FE L1185-L1219).
The function returns `true` when the searchable metadata of the two payloads is equal under the rules in section 3.
It has no side effects, performs no I/O, reads no shared state, and does not log.

The doc comment at IE L190-L194 states the intended contract: record ciphertext and key material are excluded, and the identifier, every SEM term, and the SteVec array marker are compared.

The return value is the last gate before an inbound storage payload replaces the Proxy-produced value at its position.
On `true`, the caller writes the *inbound* payload, not the derived one, into the output slot (FE L1238).
On `false`, the caller returns `EncryptError::InvalidInboundEqlPayload` (FE L1230-L1237).

## 2. Inputs and who controls them

| Input | Origin | Controlled by |
|---|---|---|
| `inbound: &EqlCiphertext` | `serde_json::from_value` at IE L61-L62, after the shape test at IE L57-L59, then `validate_storage_metadata` at IE L63 | Authenticated client: every field of the JSON object in a bind parameter (`rewrite/bind.rs` L129-L156) or a SQL literal (FE L646-L660) |
| `derived: EqlCiphertext` | `Context::encrypt(plaintexts, &verification_columns)` at FE L1207-L1219, unwrapped from `Some(EqlOutput::Store(_))` at FE L1222 | Proxy plus the black-box `encrypt_eql_v3` (ZK L354); a function of the decrypted plaintext, the `Column` captured for that position (FE L1172, L1203-L1206), and the keyset in effect when `Context::encrypt` reads it (CTX L1658) |

`EqlCiphertext` is `cipherstash_client::eql::EqlCiphertextV3` (`lib.rs` L23-L25).
It has at least the variants `Encrypted` and `SteVec` (IE L197, L205).
Whether it has further variants is not visible; the match at IE L196-L220 ends in a wildcard arm.

Field sets, from exhaustive literals:

- `EncryptedPayloadV3`: `version`, `identifier`, `ciphertext`, `hmac_256`, `bloom_filter`, `ore_block_u64_8_256`, `ope_cllw` (IE L316-L331).
- `EncryptedRecord` (the `ciphertext` field): `iv`, `ciphertext`, `tag`, `descriptor`, `keyset_id`, `decryption_policy` (IE L319-L326).
- `SteVecPayloadV3`: `version`, `kind`, `identifier`, `key_header`, `ste_vec` (IE L348-L365).
- `KeyHeader`: `iv`, `tag`, `descriptor`, `keyset_id`, `decryption_policy` (IE L352-L358).
- `SteVecEntryV3`: `selector`, `ciphertext`, `is_array`, `term` (IE L359-L364; ZK L521-L526).

## 3. Block-by-block

### 3.1 Variant dispatch (L196, L219)

The function matches on the pair `(inbound, derived)`.
Two arms accept: both `Encrypted` (L197) and both `SteVec` (L205).
Every other combination returns `false` (L219).
That includes a scalar inbound against a SteVec derived value and the reverse.

### 3.2 Scalar arm (L197-L204)

Six conjuncts joined by short-circuit `&&`, evaluated in this order:

1. `inbound.version == derived.version` (L198).
2. `inbound.identifier == derived.identifier` (L199).
3. `inbound.hmac_256 == derived.hmac_256` (L200).
4. `bloom_filters_match(&inbound.bloom_filter, &derived.bloom_filter)` (L201).
5. `inbound.ore_block_u64_8_256 == derived.ore_block_u64_8_256` (L202).
6. `inbound.ope_cllw == derived.ope_cllw` (L203).

Conjuncts 1, 2, 3, 5 and 6 use the `PartialEq` implementations of the field types, which live in `cipherstash-client`.
The term fields are `Option`s (IE L327-L330 set each to `None`; IE L255-L258 call `.is_some()` on each), so presence is part of the comparison: `Some(x)` against `None` is unequal.

The one field of `EncryptedPayloadV3` that is not compared is `ciphertext`, the `EncryptedRecord` (field list at IE L316-L331 against the conjuncts at L198-L203).
That leaves `iv`, `ciphertext`, `tag`, `descriptor`, `keyset_id` and `decryption_policy` outside this function.

### 3.3 `bloom_filters_match` (L233-L248)

- `(Some, Some)`: both `Vec<i16>` are cloned, sorted with `sort_unstable`, and compared with `==` (L235-L244).
- `(None, None)`: `true` (L245).
- Mixed presence: `false` (L246).

The comparison is multiset equality, not set equality.
Sorting does not deduplicate, so `[1, 1, 2]` and `[1, 2]` are unequal, and `[1, 2, 2]` and `[1, 1, 2]` are unequal.
The comment at L236-L238 describes the positions as a set.

The inputs are not modified; the clones are local (L239-L240).
The order of positions in `inbound.bloom_filter` is therefore preserved in the value the caller stores (FE L1238).

### 3.4 SteVec arm (L205-L218)

Conjuncts, in order:

1. `inbound.version == derived.version` (L206).
2. `inbound.identifier == derived.identifier` (L207).
3. `inbound.ste_vec.len() == derived.ste_vec.len()` (L208).
4. A positional `zip` of `inbound.ste_vec.iter()` with `derived.ste_vec` (moved), where every pair must satisfy (L209-L217):
   - `inbound.selector == derived.selector` (L214);
   - `inbound.is_array == derived.is_array` (L215);
   - `ste_vec_terms_match(&inbound.term, &derived.term)` (L216).

Conjunct 3 runs before the `zip`, so the `zip` never truncates a longer side: when lengths differ the `&&` chain stops at L208.

The comparison is positional.
Entry `n` of `inbound` is compared with entry `n` of `derived`; no matching by selector takes place.
A permutation of the same entries is unequal.

`is_array` is `Option<bool>` (IE L347, L362) and is compared as an `Option` (L215).
`Some(false)` and `None` are unequal.

Two empty `ste_vec` vectors satisfy conjuncts 3 and 4 (`all` over an empty iterator is `true`).
The function itself places no lower bound on the entry count.

Fields of `SteVecPayloadV3` that are not compared: `kind` and `key_header` (field list at IE L348-L365 against L206-L217).
Field of `SteVecEntryV3` that is not compared: `ciphertext` (field list at IE L359-L364 against L214-L216).

### 3.5 `ste_vec_terms_match` (L223-L231)

Both `Option<SteVecEntryTermV3>` values are serialized with `serde_json::to_value` (L227).
When both serializations succeed, the result is `serde_json::Value` equality (L228).
When either serialization fails, the result is `false` (L229).

The comparison is therefore over the *serialized* form of the term, not over the in-memory value.
Two distinct in-memory terms that serialize to the same JSON compare equal.
`None` serializes to JSON `null`; a `Some(term)` whose `Serialize` implementation produced `null` would compare equal to `None`.
The `Serialize` implementation of `SteVecEntryTermV3` is in `cipherstash-client` and was not read.

## 4. What the caller does with the result

`Frontend::merge_inbound_eql` (FE L1156-L1241) is reached from `encrypt_literals` (FE L674) and `encrypt_params` (FE L1123).

Sequence for each `InboundEql::Store` position:

1. The position must have a `Column`, or the call fails (FE L1169-L1171).
2. All collected ciphertexts are decrypted with `Context::decrypt_inbound_eql` (FE L1185-L1197), which reads the effective keyset (CTX L1689) and calls `ZeroKms::decrypt_eql` with `SteVecAuthentication::AllEntries` (ZK L377-L384).
3. The plaintexts are re-encrypted with `Context::encrypt` against the same `Column` values (FE L1203-L1219), which reads the effective keyset again (CTX L1658).
4. Each derived value must be `Some(EqlOutput::Store(_))` (FE L1222-L1229).
5. `sem_terms_match(&ciphertext, derived)` must be `true` (FE L1230-L1237).
6. `encrypted[index] = Some(EqlOutput::Store(ciphertext))` stores the inbound payload (FE L1238).

Both failure modes at steps 4 and 5 log the same message and return the same error (FE L1223-L1228, L1231-L1236).

After the merge, `project_query_operand` converts a `Store` value in a query-operand position into a query payload with `into_query_operand` (FE L1126-L1128, L677-L686, L1522-L1533).
The stored or projected value is later serialized with `serde_json::to_value(ct)` (`rewrite/bind.rs` L337-L340).
What reaches PostgreSQL is the re-serialization of the deserialized `inbound` struct, not the client's original bytes.

## 5. Invariants

1. Mixed variants never match (L219).
2. In the scalar arm a `true` result implies equality of `version`, `identifier`, `hmac_256`, `ore_block_u64_8_256`, `ope_cllw` under their `PartialEq` impls, and multiset equality of `bloom_filter` (L198-L203, L235-L246).
3. In the SteVec arm a `true` result implies equal `version`, equal `identifier`, equal entry count, and per-position equality of `selector`, `is_array`, and the JSON serialization of `term` (L206-L217, L227-L229).
4. The entry `zip` at L212 runs only when the entry counts are equal (L208 precedes it in the `&&` chain).
5. Presence of each optional scalar term is part of the comparison (L200-L203, L245-L246).
6. A serialization failure of either SteVec term yields `false` (L229).
7. The function does not mutate `inbound`; it takes `&EqlCiphertext` (L195) and `bloom_filters_match` sorts clones (L239-L242).
8. On a `true` result the caller stores `inbound`, so every field that this function does not compare reaches the output unchanged from what the client's JSON deserialized to (FE L1238).

## 6. Assumptions and where each is established

### 6.1 `derived` was computed from the plaintext that `inbound` decrypts to

Established by FE L1185-L1219: the `plaintexts` vector returned by `decrypt_inbound_eql` is passed directly to `Context::encrypt`.
Pairing is positional through `positions.into_iter().zip(derived)` (FE L1221).
`Context::encrypt` returns a vector whose length equals the plaintext count or fails (CTX L1659, L1667-L1673).
`ZeroKms::decrypt_eql` builds its result with length `ciphertexts.len()` (ZK L445, L461).
`place_ciphertexts` writes each output at the index of its plaintext (ZK L497-L514).

### 6.2 `derived` is a `Store` payload

Established by the `let ... else` at FE L1222-L1229.
`ZeroKms::encrypt` selects `EqlOperation::Store` only for `EqlTermVariant::Full | Partial | Tokenized` (ZK L264-L266), or for the JSON term variants when the column has no SteVec index (ZK L277, L288, L299, L316).
For `JsonPath`, `JsonAccessor`, `JsonOrd`, `JsonValueSelector` on a SteVec column the operation is `Query` (ZK L269-L316), and `JsonExtracted` is an error (ZK L323-L325).
`validate_storage_metadata` (IE L94-L125) does not consult `column.eql_term`, so an inbound storage payload in such a position reaches the merge and is rejected at FE L1222, before `sem_terms_match` runs.

### 6.3 SEM term derivation is reproducible

`sem_terms_match` returns `true` for a client-encrypted payload only when `encrypt_eql_v3` yields, for the same plaintext, column config and keyset, the same `hmac_256`, the same multiset of bloom positions, the same `ore_block_u64_8_256`, the same `ope_cllw`, and for SteVec the same entry count, entry order, selectors, array markers and terms.
Established by: nothing in this repository; it rests on `encrypt_eql_v3` (ZK L354), a black box.
The comment at IE L236-L238 records that bloom position order is *not* reproducible, which is why that field alone is order-normalized.
Integration coverage exercises `unique` + `ope` + `match` (`packages/cipherstash-proxy-integration/src/inbound_ciphertext.rs` L52-L58, L161-L184) and SteVec (L187-L213).
No test in that file configures an ORE term, so reproducibility of `ore_block_u64_8_256` is not exercised there.

### 6.4 The derived payload was built under the same keyset that authenticated the inbound ciphertext

`decrypt_inbound_eql` reads the keyset at CTX L1689 and `encrypt` reads it again at CTX L1658; each read takes the `protocol_state` read lock separately (CTX L1640-L1643).
Nothing found that holds one keyset value across both calls.
`ZeroKms::decrypt_eql` rejects, in `AllEntries` mode, any ciphertext whose recorded `keyset_id` differs from the cipher's (ZK L401-L408, L485-L490).
`sem_terms_match` itself has no keyset input.

### 6.5 Uncompared scalar field `ciphertext` (the `EncryptedRecord`) is authentic

Established outside this function:

- `descriptor` equals `"{table}/{column}"` of the destination (IE L107-L114).
- `keyset_id` equals the active cipher's keyset (ZK L401-L408).
- `iv`, `ciphertext`, `tag`, `decryption_policy`: `cipher.decrypt` (ZK L452-L455), a black box; the comment at ZK L410-L412 states the policy MAC is verified during key retrieval.

### 6.6 Uncompared SteVec field `key_header` is authentic

Established outside this function:

- `descriptor` (IE L110-L114) and `keyset_id` (ZK L488, L401-L408) as above.
- `iv`, `tag`, `decryption_policy`: used to build every entry record through `key_header.record_with_selector` (ZK L432-L436) and passed to `cipher.decrypt` (ZK L452-L455), a black box.

### 6.7 Uncompared SteVec field `kind`

Nothing found.
`kind` is not read in `sem_terms_match` (L205-L218), in `validate_storage_metadata` (IE L94-L125), or in `ZeroKms::decrypt_eql` (ZK L421-L442).
A repository-wide search for `SteVecKind` finds only the test literal at IE L350.
The set of values `kind` can deserialize to is defined in `cipherstash-client`.

### 6.8 Each non-root SteVec entry's `ciphertext` corresponds to the root plaintext

`decrypt_eql` yields a plaintext only for entry index 0 of each document (ZK L437, L459-L466); non-root entries are decrypted and discarded.
The derived payload is therefore a function of the root entry's plaintext alone.
`sem_terms_match` compares `selector`, `is_array` and `term` for every entry (L214-L216) and never compares `ciphertext`.
What a non-root entry's ciphertext decrypts to is not compared with anything in the Proxy.
The only check on it is that `cipher.decrypt` accepts the record built from the document's `key_header`, the entry ciphertext and the decoded selector (ZK L430-L436, L452-L455); what that acceptance binds is inside the black box.

### 6.9 `inbound` already passed `validate_storage_metadata`

Established at IE L63 for every `InboundEql::Store` produced by `parse`.
That function enforces `version == EQL_SCHEMA_VERSION_V3` and `identifier == column.identifier` (IE L98-L102), term presence matching the column config for scalars (IE L263-L288), and a non-empty `ste_vec` on a SteVec-configured column (IE L119).
`sem_terms_match` is `pub` and does not re-check any of this; the empty-`ste_vec` case in section 3.4 is excluded only by IE L119 and by `SteVecMissingRootEntry` at ZK L476-L478.
The derived `identifier` is `col.identifier.clone()` (ZK L330), so conjuncts L199 and L207 repeat the check at IE L99 against the same `Column`.

### 6.10 JSON fields outside the struct definitions do not survive

The stored value is the re-serialized struct (`rewrite/bind.rs` L337-L340), so a key in the client's JSON reaches PostgreSQL only if deserialization captured it into a struct field.
Whether `EncryptedPayloadV3`, `SteVecPayloadV3` or `SteVecEntryV3` reject, ignore, or capture unknown keys is defined in `cipherstash-client`; nothing found in this repository.

## 7. Calls

| Callee | Kind | What `sem_terms_match` depends on it for |
|---|---|---|
| `bloom_filters_match` (IE L233-L248) | internal | Multiset equality of bloom positions with presence equality; read in full. |
| `ste_vec_terms_match` (IE L223-L231) | internal | Equality of SteVec terms via their JSON serialization; `false` on serialization failure; read in full. |
| `serde_json::to_value::<Option<SteVecEntryTermV3>>` (IE L227) | external black box (the `Serialize` impl is in `cipherstash-client`) | That distinct terms serialize to distinct JSON, and that `Some(term)` never serializes to `null`. Not verified. |
| `PartialEq` for `Identifier`, and for the types of `version`, `hmac_256`, `ore_block_u64_8_256`, `ope_cllw`, `selector` (IE L198-L203, L206-L207, L214) | external black box | That `==` is full structural equality over every part of each value. Not verified. |
| `<[i16]>::sort_unstable`, `Vec<i16> == Vec<i16>` (IE L241-L243) | standard library | Total order on `i16`; element-wise equality. |

Upstream of the call, and relied on for the meaning of `derived`:

| Callee | Kind | What is depended on |
|---|---|---|
| `Context::decrypt_inbound_eql` (CTX L1685-L1693) -> `ZeroKms::decrypt_eql` (ZK L388-L469) | internal, wrapping black-box `cipher.decrypt` | Authentication of every record of the inbound payload and return of the root plaintext. |
| `Context::encrypt` (CTX L1647-L1675) -> `ZeroKms::encrypt` (ZK L240-L361) -> `encrypt_eql_v3` (ZK L354) | internal, wrapping black box | Canonical SEM terms for the plaintext and column. |

## 8. Shared state

None.
The function reads only its two arguments.

## 9. Test coverage observed

Unit tests in IE cover: an `hmac_256` mismatch (L426-L436), bloom order insensitivity (L438-L450), bloom presence mismatch (L452-L462), and an `is_array` mismatch (L464-L470).
No unit test in IE exercises: variant mismatch, `version` or `identifier` mismatch, `ore_block_u64_8_256` or `ope_cllw` mismatch, SteVec length mismatch, selector mismatch, SteVec term mismatch, entry reordering, or duplicate bloom positions.
Integration tests cover a spliced SteVec `op` term (`packages/cipherstash-proxy-integration/src/inbound_ciphertext.rs` L466-L508), a tampered array marker (L434-L464), and spliced scalar terms (L629-L660).

## 10. Open questions

- unclear; need to inspect `cipherstash-client` 0.42.2 `eql` module for the `Serialize` impl of `SteVecEntryTermV3`, to learn whether serialization is injective and whether any variant serializes to `null`.
- unclear; need to inspect `cipherstash-client` for the `PartialEq` impls of `Identifier` and of the `hmac_256`, `ore_block_u64_8_256`, `ope_cllw` and `selector` field types.
- unclear; need to inspect `cipherstash-client` for the variants of `SteVecKind`, how `k` deserializes, and what consumes `kind` downstream, since no Proxy code reads it.
- unclear; need to inspect `cipherstash-client` for whether `EqlCiphertextV3` has variants beyond `Encrypted` and `SteVec`.
- unclear; need to inspect `encrypt_eql_v3` for whether `ore_block_u64_8_256` and `ope_cllw` are deterministic for a fixed plaintext, config and keyset.
- unclear; need to inspect `encrypt_eql_v3` for whether SteVec entry order is deterministic and whether the root entry is always at index 0 of the derived payload.
- unclear; need to inspect `KeyHeader::record_with_selector` and `ScopedCipher::decrypt` for what an accepted non-root entry binds: the selector, the document's `key_header`, both, and whether the entry's plaintext is constrained by anything.
- unclear; need to inspect the `Deserialize` impls of `EncryptedPayloadV3`, `SteVecPayloadV3` and `SteVecEntryV3` for handling of unknown JSON keys and of keys from the other variant (for example a scalar payload that also carries `sv`).
- unclear; need to inspect `EqlCiphertextV3::into_query_operand` for which fields of a verified inbound payload it carries into a query operand (FE L1529).
- unclear; need to inspect whether the effective keyset can change between the read at CTX L1689 and the read at CTX L1658 while `merge_inbound_eql` is suspended at an `await`.
