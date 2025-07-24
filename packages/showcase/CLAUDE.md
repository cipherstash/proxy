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

Searchable encryption allows queries over encrypted data so long as the data has enabled the appropriate encrypted search configuration for a column.

The following searchable encryption strategies are supported:

### match

Provides support for text search operations over encrypted data. Enables use of the SQL `LIKE`, `NOT LIKE`, `ILIKE`, `NOT ILIKE` keywords on encrypted text.

Operators:

- `~~` (same as `LIKE`)
- `!~~` (same as `NOT LIKE`)
- `~~*` (same as `ILIKE`)
- `!~~*` (same as `NOT LIKE`)

### ore

Provides support for compare & equality operators on encrypted data.

- `<`  (less than)
- `<=` (less than or equal)
- `=`  (equal)
- `<>` (not equal)
- `>`  (greater than)
- `>=` (greater than or equal)

This implies that the built-in SQL functions `MIN` and `MAX` work on encrypted columns when they have ORE enabled.

### unique

Provides support for equality testing of encrypted columns (`=`).

This strategy is poorly named because it does NOT imply that there is a unique constraint.

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

When an `ste_vec` is created for a JSON document it allows the following operations to be performed:

- Containment operations (`@>` & `<@`)
- Fields or array elements extracted using `->` or `json_query_path`
  - support containment operations (`@>` & `<@`)
  - Strings, numbers and booleans support equality and comparison using `<`, `<=`, `=`, `<>`, `>`, `>=`.

## Referential integrity

When generating test data Claude must pay close attention to keeping the UUID foreign keys consistent between different tables and those contained in encrypted columns. Primary keys must always be UUIDs.

## Known limitations of Proxy/EQL

Data can only be encrypted or decrypted by CipherStash Proxy. The database (Postgres) does not have access to any encryption keys.

Computations on encypted data in a SQL query is not possible (except for specific operators and functions permitted by encrypted search configuration for a column) because the database cannot decrypt the data.

For example `SELECT LOWER(some_encrypted_column) FROM encrypted_table` cannot work because `some_encrypted_column` is encrypted.

**IMPORTANT: CAST operations cannot work on encrypted data** because casting would require decryption of the encrypted data within the database, which is impossible. When a column has an `ste_vec` configuration, comparison and ordering operations work directly on the encrypted values without requiring CAST operations.

When generating tests, it is important that Claude understands the fundamental limitations of EQL so that it does not generate test cases or example code that can never work.

### JSON operator limitations

The `->` cannot be chained due to a fundamental limitation in the searchable encryption. This limitation will be lifted in a future release. In the meantime `json_path_query` can be used with a JSONPath selector to select arbitrarily deeply nested values.


## Test generation

Tests must use the most concise/minimal SQL required in order to demonstrate an EQL feature, but only when minimalism is not detract from the clarity of the example.