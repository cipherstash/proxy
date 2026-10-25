# Errors

## Table of contents

- Authentication errors:
  - [Database](#authentication-failed-database)
  - [Client](#authentication-failed-client)
  - [ZeroKMS](#zerokms-authentication-failed)

- Mapping errors:
  - [Invalid parameter](#mapping-invalid-parameter)
  - [Invalid SQL statement](#mapping-invalid-sql-statement)
  - [Unsupported parameter type](#mapping-unsupported-parameter-type)
  - [Statement could not be type checked](#mapping-statement-could-not-be-type-checked)
  - [Dependent statement after DDL](#mapping-dependent-statement-after-ddl)
  - [Dependent statement after settings](#mapping-dependent-statement-after-settings)
  - [Prepared settings changed](#mapping-prepared-settings-changed)
  - [Unmodelled DDL](#mapping-unmodelled-ddl)
  - [Unmappable encrypted column](#mapping-unmappable-encrypted-column)
  - [Unresolvable table](#mapping-unresolvable-table)
  - [Ambiguous table](#mapping-ambiguous-table)
  - [COPY into encrypted column](#mapping-copy-into-encrypted-column)
  - [COPY into unknown column](#mapping-copy-into-unknown-column)
  - [COPY with a statement that uses an encrypted column](#mapping-copy-with-encrypted-statement)
  - [Unsupported encrypted operation](#mapping-unsupported-encrypted-operation)
  - [Unknown prepared statement](#mapping-unknown-prepared-statement)
  - [Internal Error](#mapping-internal-error)

- Encrypt errors:
  - [Invalid encrypted value](#encrypt-invalid-inbound-eql-payload)
  - [Column could not be encrypted](#encrypt-column-could-not-be-encrypted)
  - [Could not decrypt data for keyset](#encrypt-could-not-decrypt-data-for-keyset)
  - [Invalid keyset setting value](#encrypt-invalid-keyset-setting-value)
  - [Invalid mapping setting value](#encrypt-invalid-mapping-setting-value)
  - [Unknown CipherStash setting](#encrypt-unknown-cipherstash-setting)
  - [KeysetId could not be parsed](#encrypt-keyset-id-could-not-be-parsed)
  - [KeysetId could not be set](#encrypt-keyset-id-could-not-be-set)
  - [KeysetName could not be set](#encrypt-keyset-name-could-not-be-set)
  - [Plaintext could not be encoded](#encrypt-plaintext-could-not-be-encoded)
  - [Unknown column](#encrypt-unknown-column)
  - [Unknown Keyset Identifier](#encrypt-unknown-keyset)
  - [Unknown table](#encrypt-unknown-table)
  - [Unknown index term](#encrypt-unknown-index-term)
  - [Column configuration mismatch](#encrypt-column-config-mismatch)
  - [Missing encrypt configuration](#encrypt-missing-encrypt-configuration)
  - [Unexpected SET keyset](#encrypt-unexpected-set-keyset)
  - [Encrypted jsonb column configured for ORE ordering](#encrypt-ste-vec-ore-mode-unsupported)
  - [Encryption result count mismatch](#encrypt-result-count-mismatch)

- Decrypt errors:
   - [Column could not be deserialised](#encrypt-column-could-not-be-deserialised)
   - [Encrypted jsonb value has no root entry](#encrypt-ste-vec-missing-root-entry)
   - [Encrypted jsonb entry has an invalid selector](#encrypt-ste-vec-selector-invalid)

- Configuration errors:
  - [Missing or invalid TLS configuration](#config-missing-or-invalid-tls)
  - [Network configuration change requires restart](#config-network-change-requires-restart)

<!-- ---------------------------------------------------------------------------------------------------- -->

## Internal PostgreSQL protocol error <a id='protocol-internal-error'></a>

CipherStash Proxy encountered an unexpected internal protocol state while processing a PostgreSQL response.

### Error message

```
CipherStash Proxy encountered an internal PostgreSQL protocol error.
```

### How to fix

Retry the operation. If the error persists, collect the proxy logs and report the issue to CipherStash support. The logs retain the detailed internal error needed for diagnosis.

<!-- ---------------------------------------------------------------------------------------------------- -->

# Authentication errors


## Database <a id='authentication-failed-database'></a>

Authentication failed when connecting to the database.


### Error message

```
Database authentication failed. Check username and password
```

### How to fix

1. Check the configured username and password are correct and can connect to the database.

2. Check the database is using a supported authentication method.

CipherStash Proxy supports several PostgreSQL password authentication methods:

 - password
 - md5
 - scram-sha-256

 See [PostgreSQL password authentication](https://www.postgresql.org/docs/17/auth-password.html)


<!-- ---------------------------------------------------------------------------------------------------- -->



## Client <a id='authentication-failed-client'></a>

Authentication failed when connecting a client to the proxy.


### Error message

```
Client authentication failed. Check username and password.
```

### How to fix

1. Check the configured username and password are correct.



<!-- ---------------------------------------------------------------------------------------------------- -->


## ZeroKMS  <a id='zerokms-authentication-failed'></a>

Authentication failed when connecting to ZeroKMS.


### Error message

```
ZeroKMS authentication failed. Check the configured credentials.
```

### How to Fix

1. Check that the configured `client` credentials are correct.
2. Check that the active `keyset_name` or `keyset_id` is associated with a keyset in the configured workspace.


<!-- ---------------------------------------------------------------------------------------------------- -->


# Mapping errors


## Invalid parameter <a id='mapping-invalid-parameter'></a>

The column parameter value is not of the correct `cast` type for the target encrypted column.


### Error message

```
Invalid parameter for column 'column_name' of type 'cast' in table 'table_name'. (OID 'oid')
```


### Notes

An encrypted column definition includes the target `cast` type of the data to be stored.

To encrypt a statement parameter or literal, CipherStash Proxy decodes and casts the data into the target type.

The error indicates that the passed data cannot be decoded and cast into the expected type.

In some cases, parameter types can be converted.
For example PostgreSQL `INT2`, `INT4` and `INT8` will all be converted into encrypted `SmallInt`, `Int`, or `BigInt` types.


### How to fix

Check the parameter or literal is of the appropriate type for the configured encrypted column.


<!-- TODO: Link to encrypted types -->


<!-- ---------------------------------------------------------------------------------------------------- -->

## Invalid SQL Statement <a id='mapping-invalid-sql-statement'></a>

The SQL statement could not be parsed.

### Error message

Error messages will vary depending on the specific syntax error in the sql statement provided.


```
   sql parser error: Expected: SELECT, VALUES, or a subquery in the query body
```


### Notes

As SQL is a vast, sprawling language, the proxy may fail to parse some valid SQL statements.
Please contact CipherStash if you think your SQL is correct and the parser is wrong.


### How to fix

Check the SQL is a valid PostgreSQL SQL statement.


<!-- ---------------------------------------------------------------------------------------------------- -->


## Unsupported Parameter type <a id='mapping-unsupported-parameter-type'></a>

The parameter type is not supported.


### Error message

```
Encryption of EQL column {column_type} using strategy {eql_term} is not supported.
```

### How to fix

Check the supported types for encrypted columns.

<!-- TODO: link to doc -->



<!-- ---------------------------------------------------------------------------------------------------- -->


## Statement could not be type checked <a id='mapping-statement-could-not-be-type-checked'></a>

An error occurred when attempting to type check the SQL statement.

### Error message

```
Statement could not be type checked: '{type-check-error-message}'
```

### Notes

CipherStash Proxy checks SQL statements against the database schema to transparently encrypt and decrypt data.

The behaviour of Proxy depends on the `mapping_errors_enabled` configuration.

When `mapping_errors_enabled` is `false` (the default), then type check errors are logged, and a read-only statement is passed through to the database.
A statement that writes data (`INSERT`, `UPDATE`, `DELETE` or `MERGE`, including one inside `PREPARE`, `EXPLAIN`, `COPY (...) TO` or a `WITH` clause) is refused instead whenever any column is encrypted, because Proxy cannot tell whether it writes plaintext to an encrypted column.
This applies to every table, including tables with no encrypted columns, because Proxy cannot tell which columns a write it cannot type check reaches.
For example, these statements are refused while any column is encrypted:

- `UPDATE users SET name = v.name FROM (VALUES (1, 'Ada')) AS v(id, name) WHERE users.id = v.id`
- `INSERT INTO users (id) SELECT x FROM unnest(ARRAY[1, 2]) AS x`
- `INSERT INTO names SELECT relname FROM pg_catalog.pg_class`

Proxy type checks writes to the connection's native temporary tables, and to tables that encryption-neutral DDL earlier in the same simple query creates or alters, so those are not refused.
`CREATE TABLE IF NOT EXISTS` is the exception: the table may already exist with encrypted columns that Proxy has not loaded, so a write after it in the same simple query is refused unless an earlier statement created the table.
In a simple query, a statement is also refused when another statement in the same query needs encrypting.

When `mapping_errors_enabled` is `true`, then type check errors are raised, and statement execution halts.

Configure this setting with the environment variable `CS_DEVELOPMENT__ENABLE_MAPPING_ERRORS` or in the TOML config file under `[development] enable_mapping_errors = true`.

In our experience, most production systems have a relatively small number of columns that require protection.
As SQL is large and complex, instead of blocking statements with type check errors that are false negatives, the default behaviour of Proxy is to allow the statement.

However, this does mean it is possible that a read-only statement that references encrypted columns cannot be type-checked, and it will be passed through to the database.
When a statement is passed through to the database, the database's column constraints (provided by EQL) will catch the statement, and return a PostgreSQL error.

Example constraint error:
```sql
ERROR:  Encrypted column missing version (v) field: 34234
CONTEXT:  PL/pgSQL function _cs_encrypted_check_v(jsonb) line 6 at RAISE
SQL function "cs_check_encrypted_v1" statement 1
```

### How to fix

In most cases, this error will occur if the statement contains invalid or unsupported syntax.

Check if you are running the latest version of CipherStash Proxy, and update to the latest version if not.

If the error persists, please contact CipherStash [support](https://cipherstash.com/support).



<!-- ---------------------------------------------------------------------------------------------------- -->


## Dependent statement after DDL <a id='mapping-dependent-statement-after-ddl'></a>

A simple-query batch contains a schema-dependent statement after DDL that may change encryption
metadata. Proxy cannot observe the DDL execution result between statements in one simple-query
message, so it refuses the complete batch before PostgreSQL executes any part of it. Native DDL
and native temporary-table batches continue to pass through because they introduce no encryption
obligation.

### How to fix

Send the DDL and the dependent statement as separate queries. Extended-protocol clients may
pipeline them; Proxy defers dependent mapping until PostgreSQL reports the DDL outcome.


<!-- ---------------------------------------------------------------------------------------------------- -->


## Dependent statement after settings <a id='mapping-dependent-statement-after-settings'></a>

A simple-query batch changes CipherStash settings before a statement whose mapping depends on those settings.
Proxy must know that the settings command succeeded before it encrypts the dependent statement.
It refuses the batch rather than use an unconfirmed keyset or mapping mode.
Transaction-control commands are relevant only when they can change the effective keyset or mapping mode.
Ordinary transaction and reset batches that preserve these settings are allowed.

### How to fix

Send the settings or transaction-control command and the dependent statement as separate queries.
Wait for the command to succeed before preparing or binding encrypted values under the new settings.


<!-- ---------------------------------------------------------------------------------------------------- -->


## Prepared settings changed <a id='mapping-prepared-settings-changed'></a>

The prepared statement or bound values no longer match confirmed connection settings.
This can happen after a keyset or mapping-mode change, including one caused by transaction rollback.
Proxy refuses reuse to avoid forwarding plaintext or values encrypted under stale settings.

### How to fix

The error uses SQLSTATE `26000`, allowing drivers such as PostgreSQL JDBC to invalidate cached preparations.
Driver retry behavior still depends on transaction state and configuration.

Select the intended keyset and mapping mode, close the old statement, then prepare and bind it again.
Close, prepare, and bind may be pipelined.
Prefer bound parameters over embedded encrypted literals when a prepared statement must be reused across keysets.


<!-- ---------------------------------------------------------------------------------------------------- -->


## Unmodelled DDL <a id='mapping-unmodelled-ddl'></a>

PostgreSQL successfully executed a schema change whose connection-local effect Proxy cannot model
safely. Examples are conditional or cascading DDL, DDL that targets a table in a schema other
than the one Proxy models, and a temporary table that can shadow an encrypted table.
Schema-dependent statements are refused until the change is provably gone.

### How to fix

For a transaction-scoped change, do one of the following:

- Roll back to a savepoint that you set before the schema change.
- Roll back the transaction.
- Commit the transaction and wait for Proxy to publish an authoritative catalog snapshot.

A connection-local change, such as a temporary table that can shadow an encrypted table, is not
cleared by rollback or by catalog publication, because Proxy cannot prove from the shared catalog
that the object is gone. Roll back to a savepoint that you set before the change, or open a new
connection.


<!-- ---------------------------------------------------------------------------------------------------- -->


## Unmappable encrypted column <a id='mapping-unmappable-encrypted-column'></a>

A table referenced by the statement has a column declared with the legacy EQL v2
`eql_v2_encrypted` type. This version of Proxy targets EQL v3 and cannot encrypt or
decrypt that type, so it refuses the statement rather than serve the column.

### Error message

```
Column '{table}.{column}' is declared as 'eql_v2_encrypted', a legacy EQL v2 type that this
version of CipherStash Proxy cannot encrypt or decrypt. Statements referencing '{table}' are
refused so that plaintext is never written to the column. Migrate '{table}.{column}' to an
EQL v3 domain type.
```

### Notes

This is the signature of a partly-completed EQL v2 to v3 migration: most columns moved to
v3 domain types, and one was left behind.

Proxy identifies encrypted columns by their type. An EQL v3 encrypted column is a
`jsonb`-backed domain whose name encodes its scalar type and searchable capabilities
(`eql_v3_text_search`, `eql_v3_integer_ord`, and so on). EQL v2's `eql_v2_encrypted` carries
none of that information, so Proxy has no way to encrypt a value written to such a column,
and no way to decrypt one read back.

**Every statement referencing the table is refused, not only those naming the column.** This
is deliberate. A statement can reach a column it never names — through `SELECT *`, column
defaults, `RETURNING`, triggers or rules — and getting that analysis wrong would mean writing
plaintext into a column you believe is encrypted. Refusing on the table needs no such
analysis. Other tables are unaffected, and Proxy continues to serve them normally.

Unlike a type-check failure, this refusal is **not** subject to the `mapping_errors_enabled`
passthrough. That fallback exists for statements Proxy does not understand, on the reasoning
that they probably touch nothing encrypted. Here Proxy understands the statement exactly, and
forwarding it is the specific harm the error exists to prevent, so the statement is refused
whatever `CS_DEVELOPMENT__ENABLE_MAPPING_ERRORS` is set to.

Proxy also logs a warning naming each such column when it loads the schema at startup.

### How to fix

Migrate the column to the EQL v3 domain type matching the data it holds and the operations
you run against it — for example `eql_v3_text_search` for searchable text, or
`eql_v3_integer_ord` for an orderable integer. Existing v2 ciphertext must be decrypted and
re-encrypted as part of that migration; it cannot be reinterpreted in place.

If the column is no longer used, dropping it also resolves the error.



<!-- ---------------------------------------------------------------------------------------------------- -->


## Unresolvable table <a id='mapping-unresolvable-table'></a>

A statement that writes data refers to a table that is not in the database schema CipherStash Proxy has loaded.
Proxy cannot tell whether the statement writes to an encrypted column, so it refuses the statement rather than forward its values unencrypted.

### Error message

```
Statement refers to table 'users', which is not in the database schema Proxy has loaded, so Proxy cannot tell whether it writes to an encrypted column.
```

### Notes

The refusal applies to `INSERT`, `UPDATE`, `DELETE` and `MERGE`, including one inside `PREPARE`, `EXPLAIN`, `COPY (...) TO` or a `WITH` clause, when any column is encrypted.
It applies wherever the statement names the table, so `INSERT INTO users ... SELECT ... FROM other` is refused when `other` cannot be resolved.
Read-only statements are not refused.
When no columns are encrypted, Proxy forwards these statements.

Unlike a type-check failure, this refusal is not subject to the `CS_DEVELOPMENT__ENABLE_MAPPING_ERRORS` passthrough.

Proxy loads the tables in the schemas on its database connection's search path.
A schema-qualified name such as `public.users` resolves only to a table loaded from that schema.
A bare table name used in more than one schema on the search path cannot be resolved, because the client's search path can differ from Proxy's.
Encryption configuration is keyed by the unqualified table name, so when same-named tables in different schemas declare a column with different EQL domains, Proxy loads no configuration for that column and refuses statements on it.
When they declare the same domain, both tables share one configuration and one key descriptor, so they are not cryptographically separate: a ciphertext copied from one table to the other still decrypts, and equal values produce equal search terms in both.
A table in a schema that is not on the search path, or one created on another connection since Proxy last loaded the schema, cannot be resolved.

### How to fix

Check that the table exists and that its name is spelled as PostgreSQL resolves it.
If the table name is used in more than one schema, qualify it with its schema, as in `public.users`.
If the table is in a schema outside the search path, add that schema to the search path of the database user Proxy connects as, then restart Proxy or wait for it to reload the schema.
If the table was created on another connection, retry once Proxy has reloaded the database schema.


<!-- ---------------------------------------------------------------------------------------------------- -->


## Ambiguous table <a id='mapping-ambiguous-table'></a>

A statement that writes data refers to a table by a bare name that the connection uses for both a temporary table and a permanent table.
PostgreSQL resolves the name to the temporary table, but if that table has been dropped somewhere CipherStash Proxy cannot see, such as inside a function or a `DO` block, the name means the permanent table.
Proxy cannot tell which table the statement writes to, so it refuses the statement rather than forward its values unencrypted.

### Error message

```
Statement refers to table 'notes', which names both a temporary table and a permanent table, so Proxy cannot tell which one it writes to. Qualify the name, as in 'pg_temp.notes'.
```

### Notes

The refusal applies to the same statements as [Unresolvable table](#mapping-unresolvable-table), and only when any column is encrypted.
Read-only statements are not refused.

### How to fix

Qualify the table name: `pg_temp.notes` for the temporary table, or the permanent table's schema, as in `public.notes`.
Alternatively, give the temporary table a name no permanent table uses.

Proxy assumes PostgreSQL's default search order, in which `pg_temp` comes first.
It does not track `SET search_path`, so a client that puts `pg_temp` later in its search path should qualify table names.


<!-- ---------------------------------------------------------------------------------------------------- -->


## COPY into encrypted column <a id='mapping-copy-into-encrypted-column'></a>

A `COPY ... FROM` statement would load data into an encrypted column.
CipherStash Proxy does not encrypt `COPY` data, so it refuses the statement rather than store plaintext in the encrypted column.

### Error message

```
COPY into encrypted column 'users.encrypted_email' is not supported, as Proxy does not encrypt COPY data.
```

### Notes

The refusal applies to a `COPY ... FROM STDIN`, and to a `COPY ... FROM` a file or a program.
A `COPY` with no column list loads every column of the table, so it is refused when the table has any encrypted column.
A `COPY ... FROM` into a table with no encrypted columns, or into only the plaintext columns of a table, is forwarded to PostgreSQL with its data.
A `COPY ... TO` of a table or of a query that reads is forwarded, and returns encrypted columns as ciphertext, as Proxy does not decrypt `COPY` data.
A `COPY ... FROM` into a table Proxy cannot resolve is refused as an [Unresolvable table](#mapping-unresolvable-table) or an [Ambiguous table](#mapping-ambiguous-table).
That includes a table created with `CREATE TABLE ... (LIKE ...)` or `CREATE TABLE ... AS`, whose columns Proxy does not know.
When no columns are encrypted, Proxy does not check the table of a `COPY`.
A `COPY` after DDL that may change which columns are encrypted, in the same simple query, is still refused, as an `INSERT` would be.

### How to fix

Load the encrypted columns with `INSERT` statements through CipherStash Proxy, which encrypts the values.
Use `COPY` only for columns that are not encrypted.


<!-- ---------------------------------------------------------------------------------------------------- -->


## COPY into unknown column <a id='mapping-copy-into-unknown-column'></a>

A `COPY ... FROM` statement names a column that is not in the database schema CipherStash Proxy has loaded for its table.
Proxy cannot tell whether the column is encrypted, so it refuses the statement.

### Error message

```
COPY into column 'users.nickname', which is not in the database schema Proxy has loaded, is not supported, as Proxy cannot tell whether it is encrypted.
```

### Notes

The refusal applies only when any column is encrypted, and whatever `CS_DEVELOPMENT__ENABLE_MAPPING_ERRORS` is set to.

### How to fix

Check that the column exists and that its name is spelled as PostgreSQL resolves it.
If the column was added on another connection, retry once Proxy has reloaded the database schema.


<!-- ---------------------------------------------------------------------------------------------------- -->


## COPY with a statement that uses an encrypted column <a id='mapping-copy-with-encrypted-statement'></a>

A simple query contains a `COPY` together with a statement that uses an encrypted column.
CipherStash Proxy rewrites such a query to encrypt it, and cannot keep the `COPY` in the rewritten query.

### Error message

```
COPY must be sent in a query of its own when the query has a statement that uses an encrypted column.
```

### How to fix

Send the `COPY` in a query of its own.


<!-- ---------------------------------------------------------------------------------------------------- -->


## Unsupported encrypted operation <a id='mapping-unsupported-encrypted-operation'></a>

A statement uses an operation on an encrypted column that the column's EQL type does not support, such as `<` or `ORDER BY` on a column with no ordering index, or `LIKE` on a column with no match index.
CipherStash Proxy cannot encrypt the operand for that operation, so it refuses the statement rather than send the operand to the database as plaintext.

### Error message

```
Statement uses an operation that an encrypted column's EQL type does not support: Type `...` does not satisfy bounds `Ord`.
```

### Notes

The refusal applies to reads as well as writes, and whether or not `CS_DEVELOPMENT__ENABLE_MAPPING_ERRORS` is set.

### How to fix

Change the statement to use only operations the column supports, or change the column's EQL domain type to one that supports the operation.


<!-- ---------------------------------------------------------------------------------------------------- -->


## Unknown prepared statement <a id='mapping-unknown-prepared-statement'></a>

A Bind named a prepared statement that was not prepared with a Parse on this connection.
Proxy keeps encryption details only for statements that touch encrypted columns, so it cannot
tell whether the values bound to an unknown statement must be encrypted. It refuses the Bind
rather than send them as they are. The error uses PostgreSQL's own code for a missing prepared
statement, `26000`, and the connection stays open: PostgreSQL skips the rest of the batch until
the next Sync, as it does for any failed Bind.

### Error message

```
Prepared statement '{name}' does not exist on this connection.
```

### How to fix

- Prepare the statement with a Parse message on the same connection before binding it. Most
  drivers do this for you.
- A statement created with SQL `PREPARE name AS ...` cannot be bound through the extended
  protocol through Proxy. Use `EXECUTE name` instead, or prepare it with a Parse message.
- If a connection pool hands out connections whose prepared statements were deallocated, make
  sure the driver prepares statements again after a reset (`DEALLOCATE ALL` or `DISCARD ALL`).

The check does not apply while mapping is disabled.


<!-- ---------------------------------------------------------------------------------------------------- -->


## Internal Mapper error <a id='mapping-internal-error'></a>

An internal error occurred when attempting to type check or transform a SQL statement.
This could be due to an internal invariant failure, or because of a specific fragment of unsupported SQL syntax.

### Error message

```
Statement encountered an internal error. This may be a bug in the statement mapping module of CipherStash Proxy.
```

### How to fix

Check if you are running the latest version of CipherStash Proxy, and update to the latest version if not.

If the error persists, please contact CipherStash [support](https://cipherstash.com/support).



<!-- ---------------------------------------------------------------------------------------------------- -->


# Encrypt errors


## Invalid encrypted value <a id='encrypt-invalid-inbound-eql-payload'></a>

CipherStash Proxy rejected an application-generated EQL storage payload or
query operand because it was malformed, unauthentic, intended for another
column, carried unexpected searchable encrypted metadata (SEM), or appeared in
an invalid statement position. The response deliberately does not identify
which validation failed.

### Error message

```
Invalid encrypted value. For help visit https://github.com/cipherstash/proxy/blob/main/docs/errors.md#encrypt-invalid-inbound-eql-payload
```

### How to fix

Regenerate the payload using the same column configuration, keyset, and
credentials as Proxy. Storage payloads must target the inferred destination
column and carry ciphertext plus exactly its configured SEM terms. Query-only
payloads must be used only in query positions.

### Plaintext compatibility

On every encrypted column type, Proxy reserves three JSON object shapes for
application-generated EQL:

- a storage payload with top-level `v` and `i` keys plus at least one of `c`,
  `h`, or `sv`;
- a scalar query payload with top-level `v` and `i` keys plus at least one of
  `hm`, `bf`, `ob`, or `op`; or
- a SteVec query payload whose only top-level key is `sv`.

An object matching one of these patterns is validated as EQL rather than
encrypted as plaintext. Invalid EQL is rejected, and query-only payloads are
rejected in storage positions. There is no opt-out for this fail-closed check.

Before upgrading, audit JSON values supplied as plaintext to encrypted columns.
For a `jsonb` source column named `value`, this predicate identifies all three
reserved shapes:

```sql
WHERE CASE
  WHEN jsonb_typeof(value) <> 'object' THEN false
  ELSE (
    value ? 'v'
    AND value ? 'i'
    AND value ?| ARRAY['c', 'h', 'sv', 'hm', 'bf', 'ob', 'op']
  ) OR (
    value ? 'sv'
    AND NOT EXISTS (
      SELECT 1
      FROM jsonb_object_keys(value) AS keys(candidate_key)
      WHERE candidate_key <> 'sv'
    )
  )
END
```

For text sources, first restrict the scan to values that your application knows
are valid JSON, then apply the same predicate after casting them to `jsonb`.
Rename one of these top-level keys or generate the value as an EQL payload
before sending it through Proxy.

<!-- ---------------------------------------------------------------------------------------------------- -->


## Column could not be encrypted <a id='encrypt-column-could-not-be-encrypted'></a>

The column could not be encrypted.


### Error message

```
Column 'column_name' in table 'table_name' could not be encrypted.
```

### Notes

CipherStash Proxy uses [CipherStash ZeroKMS](https://cipherstash.com/products/zerokms) for low-latency encryption and decryption operations.

The error indicates an issue has occurred in the encryption pipeline processing.
The most likely cause is network access to the ZeroKMS service.

### How to Fix

1. Check that CipherStash ZeroKMS is available at [the status page](https://status.cipherstash.com/).
2. Check that CipherStash Proxy has network access to ZeroKMS in the appropriate region.
<!-- TODO: Link to ZeroKMS Doc -->
3. Check that the encrypted configuration `cast` matches the expected type.
<!-- TODO: Link to config -->




<!-- ---------------------------------------------------------------------------------------------------- -->


## Encryption result count mismatch <a id='encrypt-result-count-mismatch'></a>

The encryption service returned a different number of encrypted values than the number of values it was asked to encrypt.

### Error message

```
Expected 2 encrypted values but received 0.
```

### Notes

Proxy matches encrypted values to statement parameters by position. If any value were missing, the parameters after it could be sent to the database unencrypted, so Proxy refuses the statement instead.

### How to Fix

This indicates an internal error in the encryption pipeline. Retry the statement, and if the error persists, contact CipherStash support with the Proxy logs.




<!-- ---------------------------------------------------------------------------------------------------- -->


## Invalid keyset setting value <a id='encrypt-invalid-keyset-setting-value'></a>

`SET CIPHERSTASH.KEYSET_ID` or `SET CIPHERSTASH.KEYSET_NAME` was given a value Proxy does not accept.
Unsupported values are rejected instead of silently retaining the previous keyset.

### Error message

```
Setting '{setting}' requires exactly one value. Use a quoted UUID for KEYSET_ID, a keyset name (quoted, unquoted, or numeric) for KEYSET_NAME, or DEFAULT to reset.
```

### How to fix

Provide exactly one value of a supported form:

- `CIPHERSTASH.KEYSET_ID` accepts a UUID, normally quoted.
  A value that is not a valid UUID returns [KeysetId could not be parsed](#encrypt-keyset-id-could-not-be-parsed).
- `CIPHERSTASH.KEYSET_NAME` accepts a keyset name as a quoted string, an unquoted identifier, or a number.
- Either setting accepts `DEFAULT` (or `RESET`) to clear the selected keyset.

```
SET [ SESSION | LOCAL ] CIPHERSTASH.KEYSET_ID { TO | = } '{KeysetId}'
SET [ SESSION | LOCAL ] CIPHERSTASH.KEYSET_NAME { TO | = } '{KeysetName}'
SET [ SESSION | LOCAL ] CIPHERSTASH.KEYSET_ID { TO | = } DEFAULT
```


<!-- ---------------------------------------------------------------------------------------------------- -->


## Invalid mapping setting value <a id='encrypt-invalid-mapping-setting-value'></a>

`SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING` was given a value Proxy cannot interpret as a boolean.
Unsupported values are rejected instead of silently retaining the previous mapping mode.

### Error message

```
Setting 'CIPHERSTASH.UNSAFE_DISABLE_MAPPING' requires exactly one boolean value, such as true, false, on, off, yes, no, 1 or 0, or DEFAULT to reset.
```

### How to fix

Provide exactly one boolean value, quoted or unquoted.
Proxy accepts the spellings PostgreSQL accepts for a boolean, ignoring case:

- `true`, `yes`, `on` or `1` disables mapping.
- `false`, `no`, `off` or `0` enables mapping.
- A unique prefix such as `t`, `f`, `y`, `n` or `of` is also accepted.
- `DEFAULT` (or `RESET`) restores the default, with mapping enabled.

```
SET [ SESSION | LOCAL ] CIPHERSTASH.UNSAFE_DISABLE_MAPPING { TO | = } { true | false }
SET [ SESSION | LOCAL ] CIPHERSTASH.UNSAFE_DISABLE_MAPPING { TO | = } DEFAULT
```


<!-- ---------------------------------------------------------------------------------------------------- -->


## Unknown CipherStash setting <a id='encrypt-unknown-cipherstash-setting'></a>

A `SET` or `RESET` command named a `CIPHERSTASH.*` setting that Proxy does not recognise.
Proxy handles every setting in the `CIPHERSTASH` namespace itself, so it rejects an unknown name instead of ignoring it.
Setting names are case-insensitive, and quoting does not change the name: `"cipherstash.keyset_name"` and `cipherstash."KEYSET_NAME"` both name `CIPHERSTASH.KEYSET_NAME`.

### Error message

```
Setting '{setting}' is not a CipherStash setting. Use CIPHERSTASH.KEYSET_ID, CIPHERSTASH.KEYSET_NAME or CIPHERSTASH.UNSAFE_DISABLE_MAPPING.
```

### How to fix

Check the spelling of the setting name.
Proxy supports these settings:

- `CIPHERSTASH.KEYSET_ID`
- `CIPHERSTASH.KEYSET_NAME`
- `CIPHERSTASH.UNSAFE_DISABLE_MAPPING`


<!-- ---------------------------------------------------------------------------------------------------- -->


## KeysetId could not be parsed <a id='encrypt-keyset-id-could-not-be-parsed'></a>

A keyset_id could not be parsed using the `SET CIPHERSTASH.KEYSET_ID` command.

### Error message

```
KeysetId `{id}` could not be parsed using `SET CIPHERSTASH.KEYSET_ID`. KeysetId should be a valid UUID.
```

### How to Fix

1. Check that the `KeysetId` is a valid UUID.


```
   SET [ SESSION ] CIPHERSTASH.KEYSET_ID { TO | = } '{KeysetId}'
```

<!-- ---------------------------------------------------------------------------------------------------- -->


## KeysetId could not be set <a id='encrypt-keyset-id-could-not-be-set'></a>

A KeysetId could not be set using the `SET CIPHERSTASH.KEYSET_ID` command.


### Error message

```
Keyset Id could not be set using `SET CIPHERSTASH.KEYSET_ID`
```

### How to Fix

1. Check the syntax of the `SET CIPHERSTASH.KEYSET_ID` command. The `keyset_id` value should be in single quotes.
2. Check that the `keyset_id` is a valid UUID.
3. Check that the value is being set as a literal. The PostgreSQL `SET` statement does not support parameterised querying.


```
   SET [ SESSION ] CIPHERSTASH.KEYSET_ID { TO | = } '{keyset_id}'
```


<!-- ---------------------------------------------------------------------------------------------------- -->


## KeysetName could not be set <a id='encrypt-keyset-name-could-not-be-set'></a>

KeysetName could not be set using the `SET CIPHERSTASH.KEYSET_NAME` command.


### Error message

```
Keyset Name could not be set using `SET CIPHERSTASH.KEYSET_NAME`
```

### How to Fix

1. Check the syntax of the `SET CIPHERSTASH.KEYSET_ID` command. The `keyset_name` value should be in single quotes.
2. Check that the value is being set as a literal. The PostgreSQL `SET` statement does not support parameterised querying.


```
   SET [ SESSION ] CIPHERSTASH.KEYSET_NAME { TO | = } '{keyset_name}'
```


<!-- ---------------------------------------------------------------------------------------------------- -->


## Unknown keyset identifier <a id='encrypt-unknown-keyset'></a>

The specified keyset could not be loaded.


### Error message

```
Unknown keyset name or id '{keyset}'
```

### How to Fix

1. Check that the active `keyset_name` or `keyset_id` is associated with a keyset in the configured workspace.
2. Check that the configured `client` has been granted access to the keyset via the dashboard.
3. Keyset names are case sensitive. If setting the active keyset by name, check that the `keyset_name` is an exact match.
4. Check that the configured `client` credentials are correct.


<!-- ---------------------------------------------------------------------------------------------------- -->



## Could not decrypt data for keyset <a id='encrypt-could-not-decrypt-data-for-keyset'></a>

The data belonging to the active `keyset_id` could not be decrypted.


### Error message

```
Could not decrypt data for keyset '{keyset_id}'
```

### Notes

This error is caused because the active `keyset_id` does not match the `keyset_id` of the data being decrypted.

Each encrypted record belong to a `keyset` with a unique identifier (the `keyset_id`).

The proxy encrypts data in the currently active `keyset`.

A single default keyset can be defined in the proxy configuration or the `SET CIPHERSTASH.KEYSET_ID` statement can be used to dynamically set the active `keyset` at runtime.

If the `keyset_id` of the record does not match the current `keyset_id` the data cannot be decrypted.

### How to Fix

1. Check that the `keyset_id` in the configuration matches the `keyset_id` of the encrypted records.
2. If using the `SET CIPHERSTASH.KEYSET_ID` statement, check that this `keyset_id` matches the `keyset_id` of the encrypted records.
3. Check that the configured `client` has been granted access to the `keyset_id`.


<!-- ---------------------------------------------------------------------------------------------------- -->


## Plaintext could not be encoded <a id='encrypt-plaintext-could-not-be-encoded'></a>

The encrypted data in a column returned by a SQL statement cannot be encoded into the correct type.

### Error message

```
Decrypted column could not be encoded as the expected type.
```

### Notes

An encrypted column definition includes the target `cast` type of the data to be stored.

Encrypted data is stored as the raw (encrypted) `bytes` in the database.

When a statement returns encrypted data, CipherStash Proxy decrypts the data, casts, and encodes as the PostgreSQL representation of the target type.

The error indicates that the stored data cannot be encoded and returned as the expected type.

Changing the encrypted column definition of a column with existing data can cause this error.

For example:

- column is defined with a cast of `text`
- data is encrypted and stored as `text`
- column is redefined with a cast of  `int`
- error as existing records stored as `text` data cannot be decrypted, cast and encoded as `int`


### How to fix

1. Check the encrypted configuration has the correct type.
2. Check that the configuration has not changed.
3. Check [EQL](https://github.com/cipherstash/encrypt-query-language).


<!-- ---------------------------------------------------------------------------------------------------- -->


## Unknown Column <a id='encrypt-unknown-column'></a>

The column has an encrypted type (an EQL v3 encrypted domain type, e.g. `eql_v3_text_search`) with no encryption configuration.

Without the configuration, Cipherstash Proxy does not know how to encrypt the column.
Any data is unprotected and unencrypted.


### Error message

```
Column 'column_name' in table 'table_name' has no Encrypt configuration
```


### How to fix

1. Define the encrypted configuration using [EQL](https://github.com/cipherstash/encrypt-query-language).
   <!-- TODO: link to doc -->
2. Add `users.email` as an encrypted column:
   ```sql
   SELECT cs_add_column_v1('users', 'email');
   ```

<!-- ---------------------------------------------------------------------------------------------------- -->


## Unknown Table <a id='encrypt-unknown-table'></a>

The table has one or more encrypted columns (an EQL v3 encrypted domain type, e.g. `eql_v3_text_search`) with no encryption configuration.

Without the configuration, Cipherstash Proxy does not know how to encrypt the column.
Any data is unprotected and unencrypted.


### Error message

```
Table 'table_name' has no Encrypt configuration
```

### How to fix

1. Define the encrypted configuration using [EQL](https://github.com/cipherstash/encrypt-query-language).
   <!-- TODO: link to doc -->
2. Add `users.email` as an encrypted column:
   ```sql
   SELECT cs_add_column_v1('users', 'email');
   ```



<!-- ---------------------------------------------------------------------------------------------------- -->


## Unknown Index Term <a id='encrypt-unknown-index-term'></a>

The encrypted column has an unknown index configuration.

EQL validates indexes when they are added to ensure that the configuration is correctly defined.
However, if the configuration is changed directly in the database, it is possible to misconfigure the setup.


### Error message

```
Unknown Index Term for column '{column_name}' in table '{table_name}'.
```


### How to fix

1. Check the Encrypt configuration for the column.
2. Define the encrypted configuration using [EQL](https://github.com/cipherstash/encrypt-query-language).


<!-- ---------------------------------------------------------------------------------------------------- -->
<!-- ---------------------------------------------------------------------------------------------------- -->


## Column configuration mismatch <a id='encrypt-column-config-mismatch'></a>

A returned encrypted column does not match the column configuration.

### Error message

```
Column configuration for column '{column_name}' in table '{table_name}' does not match the encrypted column.
```

### Notes

CipherStash Proxy validates that encrypted columns match the configuration before decrypting any data.
If the table and column are not the same, this error is returned.
The check is there to help prevent "confused deputy" issues and the error should *never* appear during normal operation.

If the error persists, please contact CipherStash [support](https://cipherstash.com/support).


### Further reading

[AWS: The confused deputy problem](https://docs.aws.amazon.com/IAM/latest/UserGuide/confused-deputy.html)
[Wikipedia: Confused deputy problem](https://en.wikipedia.org/wiki/Confused_deputy_problem)

<!-- ---------------------------------------------------------------------------------------------------- -->


## Missing encrypt configuration <a id='encrypt-missing-encrypt-configuration'></a>

The encrypted column type does not have a matching encrypt configuration.


### Error message

```
Missing encrypt configuration for column type `{plaintext_type}`.
```

### How to fix

1. Define the encrypted configuration for the column type using [EQL](https://github.com/cipherstash/encrypt-query-language).
2. If this error persists, please contact CipherStash [support](https://cipherstash.com/support) as this may indicate a bug.


<!-- ---------------------------------------------------------------------------------------------------- -->


## Unexpected SET keyset <a id='encrypt-unexpected-set-keyset'></a>

A `SET CIPHERSTASH.KEYSET` statement was used when a default keyset has already been configured.


### Error message

```
Cannot SET CIPHERSTASH.KEYSET if a default keyset has been configured.
```

### How to fix

1. Remove the `SET CIPHERSTASH.KEYSET` statement from your application code.
2. Or remove the `default_keyset_id` from the proxy configuration to allow dynamic keyset selection.


<!-- ---------------------------------------------------------------------------------------------------- -->


## Encrypted jsonb column configured for ORE ordering <a id='encrypt-ste-vec-ore-mode-unsupported'></a>

An encrypted `jsonb` (SteVec) column is configured for Standard-mode ORE ordering, which EQL v3 does not support.

EQL v3 orders encrypted `jsonb` entries by the CLLW-OPE (`op`) term and has no representation for CLLW-ORE (`oc`). A column carried over from an earlier configuration that used ORE ordering therefore cannot be encrypted under EQL v3.


### Error message

```
An encrypted jsonb column is configured for ORE ordering, which EQL v3 does not support.
```


### How to fix

1. Reconfigure the column to use a supported ordering mode.
2. Re-encrypt the column's data under the new configuration.


<!-- ---------------------------------------------------------------------------------------------------- -->


# Decrypt errors


## Column could not be deserialised <a id='encrypt-column-could-not-be-deserialised'></a>

The column could not be deserialised for decryption.


### Error message

```
Column 'column_name' in table 'table_name' could not be deserialised.
```

### Notes

CipherStash Proxy stores encrypted data and search terms as `jsonb`. The structure is defined as part of EQL.

The error indicates an internal issue has occurred deserialising and extracting the ciphertext data for decryption.
It may be caused if the the encrypted data has been altered by another process or application.

If the error persists, please contact CipherStash [support](https://cipherstash.com/support).


### How to Fix

1. Check that the data in the encrypted column is in the correct format [EQL](https://github.com/cipherstash/encrypt-query-language).

<!-- TODO: Link to EQL Doc on storage format-->




<!-- ---------------------------------------------------------------------------------------------------- -->


## Encrypted jsonb value has no root entry <a id='encrypt-ste-vec-missing-root-entry'></a>

An encrypted `jsonb` (SteVec) value has an empty `sv` array and cannot be decrypted.

The first entry of a SteVec document (`sv[0]`) is its decryption root. A document with no entries has nothing to decrypt, which indicates the stored value has been truncated or altered by another process.


### Error message

```
Encrypted jsonb value has no root entry and cannot be decrypted.
```


### How to fix

1. Check that the data in the encrypted column has not been modified outside CipherStash Proxy.
2. If the error persists, please contact CipherStash [support](https://cipherstash.com/support).


<!-- ---------------------------------------------------------------------------------------------------- -->


## Encrypted jsonb entry has an invalid selector <a id='encrypt-ste-vec-selector-invalid'></a>

An encrypted `jsonb` (SteVec) entry has a selector that is not exactly 16 hex-encoded bytes.

A SteVec entry's selector is the source of both AEAD bindings (nonce and AAD), so it must be exactly 16 hex-encoded bytes. A selector of any other length indicates the stored value has been altered by another process.


### Error message

```
Encrypted jsonb entry has an invalid selector '{selector}'.
```


### How to fix

1. Check that the data in the encrypted column has not been modified outside CipherStash Proxy.
2. If the error persists, please contact CipherStash [support](https://cipherstash.com/support).


<!-- ---------------------------------------------------------------------------------------------------- -->

# Configuration errors


## Database <a id='config-missing-or-invalid-tls'></a>

There was a problem with the Tls configuration.


### Error message

```
# PEM-based configuration
Invalid Transport Layer Security (TLS) certificate.
Invalid Transport Layer Security (TLS) private key.

# Path-based configuration
Missing Transport Layer Security (TLS) certificate at path: {path}.
Missing Transport Layer Security (TLS) private key at path: {path}.
```

### How to fix

If using path-based configuration:
Check that the certificate and private key exists at the specified path.
Check that the certificate and private key are valid.

If using PEM-based configuration:
Check that the certificate and private key are valid.


<!-- ---------------------------------------------------------------------------------------------------- -->


## Network configuration change requires restart <a id='config-network-change-requires-restart'></a>

A configuration reload was attempted with network-level changes that require a full restart.

### Error message

```
Network configuration change requires restart
```

### Notes

When receiving a SIGHUP signal, CipherStash Proxy attempts to reload application-level configuration without disrupting active connections. However, certain network-related configuration changes require stopping and restarting the proxy service to take effect.

The following settings require a restart when changed:
- `server.host` - The host address the proxy listens on
- `server.port` - The port the proxy listens on
- `server.require_tls` - TLS requirement setting
- `server.worker_threads` - Number of worker threads
- `tls` - Any TLS certificate or key configuration

### How to fix

1. Stop the CipherStash Proxy service
2. Update the configuration as needed
3. Restart the CipherStash Proxy service

Application-level configuration changes (database, auth, encrypt, log, prometheus, development) can be reloaded without restart using SIGHUP.

<!-- ---------------------------------------------------------------------------------------------------- -->
