# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in the showcase crate.

## Showcase Overview

The showcase crate is an application that demonstrates CipherStash Proxy & EQL (Encrypt Query Language) features.

Its intended purposes are:

1. Provide a realistic example domain model that includes sensitive data that should be protected with encryption.
2. For every Postgres function and operator supported by EQL, provide working executable examples of SQL queries that   search sensitive data.
3. Be executable - and thus always up to date and correct.
4. Serve as a reference for potential customers.
5. To document limitations of EQL and differences in behaviour of how EQL functions and operators work compared to the regular Postgres equivalents.

## Searchable encryption

Searchable encryption allows queries over encrypted data so long as the column's
type declares the required capability.

**EQL v3 model — self-configuring domain types.** Unlike EQL v2 (which used an
opaque `eql_v2_encrypted` type plus an `eql_v2.add_search_config` call per
column), EQL v3 gives each `(token type × capability)` combination its own
Postgres domain type over `jsonb`. The column type alone declares both the
encryption and what can be searched — there is no separate config call. The
capability groups below map to v3 domain-name suffixes:

| Capability (v2 name)  | v3 domain suffix                          | Operations |
|-----------------------|-------------------------------------------|------------|
| match                 | `_match` (e.g. `eql_v3_text_match`)        | `LIKE`/`ILIKE`, `@@` |
| ore / comparison      | `_ord` / `_ord_ore` (e.g. `eql_v3_integer_ord`) | `<` `<=` `=` `<>` `>` `>=`, `MIN`/`MAX` |
| unique / equality     | `_eq` (e.g. `eql_v3_text_eq`)              | `=` `<>` |
| ste_vec (encrypted JSON) | `json_search` (`eql_v3_json_search`)    | `->` `->>` `@>` `<@`, `jsonb_path_*` |

The showcase uses **`eql_v3_json_search`** (the ste_vec/encrypted-JSON domain)
for its encrypted columns.

### match

Text search over encrypted text (`_match` domains). Enables `LIKE`, `NOT LIKE`,
`ILIKE`, `NOT ILIKE` and the `@@` fuzzy-match operator.

Operators:

- `~~` (same as `LIKE`)
- `!~~` (same as `NOT LIKE`)
- `~~*` (same as `ILIKE`)
- `!~~*` (same as `NOT LIKE`)
- `@@` (fuzzy match)

### ore

Compare & equality operators on encrypted scalars (`_ord` / `_ord_ore` domains).

- `<`  (less than)
- `<=` (less than or equal)
- `=`  (equal)
- `<>` (not equal)
- `>`  (greater than)
- `>=` (greater than or equal)

This implies that the built-in SQL functions `MIN` and `MAX` work on encrypted
columns typed as an ordering domain.

### unique

Equality testing of encrypted columns (`=`) via an `_eq` domain.

This capability is historically called "unique" but does NOT imply a unique
constraint.

### ste_vec

Provides support for the following JSON operators and functions on encrypted JSON data:

Operators:

- `->`  (field access)
- `->>` (field access)
- `@>`  (contains)
- `<@`  (contained by)

Functions:

- `jsonb_path_query`
- `jsonb_path_query_first`
- `jsonb_path_exists`
- `jsonb_array_length`
- `jsonb_array_elements`
- `jsonb_array_elements_text`

An `eql_v3_json_search` (ste_vec) column allows the following operations to be performed:

- Containment operations (`@>` & `<@`)
- Fields or array elements extracted using `->` or `json_query_path`
  - support containment operations (`@>` & `<@`)
  - Strings, numbers and booleans support equality and comparison using `<`, `<=`, `=`, `<>`, `>`, `>=`.

## Referential integrity

When generating test data Claude must pay close attention to keeping the UUID foreign keys consistent between different tables and those contained in encrypted columns. Primary keys must always be UUIDs.

## Known limitations of Proxy/EQL

Data can only be encrypted or decrypted by CipherStash Proxy. The database (Postgres) does not have access to any encryption keys.

Computations on encypted data in a SQL query is not possible (except for specific operators and functions permitted by encrypted search configuration for a column) because the database cannot decrypt the data.

For example `SELECT LOWER(some_encrypted_column) FROM encrypted_table` cannot work because `some_encrypted_column` is encrypted and the built-in SQL function `LOWER` can only operate on plaintext (not ciphertext).

**IMPORTANT: by default encrypted columns and encrypted literals cannot be passed as arguments to SQL functions**

Encrypted columns can only be passed as arguments to a SQL function if the value has an encrypted search index that supports that specific function.

For example, the SQL `AVG` function cannot be used on encrypted numeric values. But the SQL `MIN` and `MAX` functions can be used on an encrypted value that has an ORE index.

**IMPORTANT: CAST operations cannot work on encrypted data** because casting would require decryption of the encrypted data within the database, which is impossible. When a column is typed as an `eql_v3_json_search` (ste_vec) domain, comparison and ordering operations work directly on the encrypted values without requiring CAST operations.

When generating tests, it is important that Claude understands the fundamental limitations of EQL so that it does not generate test cases or example code that can never work.

### JSON operator chaining

**The `->` / `->>` operators CAN be chained on `ste_vec` encrypted columns**, as
long as the whole path is written in one expression. The mapper collapses the
chain into a single keyed path against the root document (an encrypted
intermediate value cannot be traversed by the database, so `pii -> 'vitals' ->
'blood_type'` is resolved as the one path `$.vitals.blood_type`, not as two
hops).

All of these work, and are equivalent:

- `pii -> 'vitals' -> 'blood_type'` ✅
- `(pii -> 'vitals') -> 'blood_type'` ✅ (parentheses are transparent)
- `pii -> 'vitals' ->> 'blood_type'` ✅ (spellings mix freely)
- `jsonb_path_query_first(pii, '$.vitals') -> 'blood_type'` ✅
- `jsonb_path_query_first(pii, '$.vitals.blood_type')` ✅ (a single path is
  always fine, and often the clearest)

Chains work in every position: projections, `WHERE` equality (`= <>` — fused
with the compared value into one containment needle), `WHERE` comparisons
(`< <= > >=`), and `ORDER BY`.

Placeholder steps work (`pii -> 'vitals' -> $1`), with one exception: a
placeholder step **followed by a literal step** (`pii -> $1 -> 'blood_type'`)
is refused, because the literal selector is encrypted before `$1` is bound.
Give the whole path via placeholders or put the literal steps first.

**Real limitations that remain:**

- **A path cannot be split across a subquery, CTE, or view.**
  `SELECT a -> 'x' FROM (SELECT pii -> 'vitals' AS a FROM patients) s` ❌ —
  the extracted value is a single encrypted entry with nothing left to
  traverse, and the root column is out of scope on the far side of the
  boundary. This is rejected with a type error, not silently wrong. Write the
  whole path in one expression instead.
- **An extracted field is not a document.** It supports equality, comparison,
  and `ORDER BY` (it carries equality and ordering terms), but it cannot be
  traversed further or used where a whole document is required.
- **Nested `jsonb_path_query*` calls are not collapsed.**
  `jsonb_path_query_first(jsonb_path_query_first(pii, '$.a'), '$.b')` ❌ —
  write the single path `'$.a.b'` instead. (Chaining `->` on top of a
  `jsonb_path_query*` root is fine, per the examples above.)


## Test generation

Tests must use the most concise/minimal SQL required in order to demonstrate an EQL feature, but only when minimalism is not detract from the clarity of the example.