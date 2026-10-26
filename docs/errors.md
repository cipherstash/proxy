# Errors

## Table of contents

- Authentication errors:
  - [Database](#authentication-failed-database)
  - [Client](#authentication-failed-client)

- Mapping errors:
  - [Invalid parameter](#mapping-invalid-parameter)
  - [Invalid SQL statement](#mapping-invalid-sql-statement)
  - [Unsupported parameter type](#mapping-unsupported-parameter-type)
  - [Statement could not be type checked](#mapping-statement-could-not-be-type-checked)
  - [COPY into encrypted column](#mapping-copy-into-encrypted-column)
  - [COPY into unknown table](#mapping-copy-into-unknown-table)
  - [COPY of a query that writes](#mapping-copy-of-query-that-writes)
  - [COPY with a statement that uses an encrypted column](#mapping-copy-with-encrypted-statement)
  - [Internal Error](#mapping-internal-error)
  - [Prepared settings changed](#mapping-prepared-settings-changed)

- Encrypt errors:
  - [Column could not be encrypted](#encrypt-column-could-not-be-encrypted)
  - [Column could not be encrypted](#encrypt-column-could-not-be-encrypted)
  - [Could not decrypt data for keyset](#encrypt-encrypt-could-not-decrypt-data-for-keyset)
  - [KeysetId could not be parsed](#encrypt-keyset-id-could-not-be-parsed)
  - [Invalid keyset setting value](#encrypt-invalid-keyset-setting-value)
  - [Invalid mapping setting value](#encrypt-invalid-mapping-setting-value)
  - [Unknown CipherStash setting](#encrypt-unknown-cipherstash-setting)
  - [KeysetId could not be set](#encrypt-keyset-id-could-not-be-set)
  - [KeysetName could not be set](#encrypt-keyset-name-could-not-be-set)
  - [Plaintext could not be encoded](#encrypt-plaintext-could-not-be-encoded)
  - [Unknown column](#encrypt-unknown-column)
  - [Unknown Keyset Identifier](#encrypt-unknown-keyset)
  - [Unknown table](#encrypt-unknown-table)
  - [Unknown index term](#encrypt-unknown-index-term)
  - [Column configuration mismatch](#encrypt-column-config-mismatch)

- Decrypt errors:
   - [Column could not be deserialised](#encrypt-column-could-not-be-deserialised)

- Configuration errors:
  - [Missing or invalid TLS configuration](#config-missing-or-invalid-tls)
  - [Network configuration change requires restart](#config-network-change-requires-restart)

<!-- ---------------------------------------------------------------------------------------------------- -->

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


## ZeroKMS  <a id='authentication-failed-zerokms'></a>

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
Encryption of PostgreSQL {name} (OID {oid}) types is not currently supported.
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

When `mapping_errors_enabled` is `false` (the default), then type check errors are logged, and the statement is passed through to the database.

When `mapping_errors_enabled` is `true`, then type check errors are raised, and statement execution halts.

When encrypted columns are configured, a statement is passed through only if it cannot write plaintext to an encrypted column, whatever the `mapping_errors_enabled` setting.
Proxy raises the type check error, and the database does not run the query, when:

- the statement writes data (an `INSERT`, `UPDATE`, `DELETE` or `MERGE`, including one in a `PREPARE`, an `EXPLAIN` or a `WITH` clause) to a table that has encrypted columns, or to a table that Proxy has not loaded, as Proxy cannot tell which columns it writes to.
  A `MERGE` is always refused.
  A write that Proxy cannot type check is passed through only when Proxy knows that every table it writes to has no encrypted columns.
- the statement is in a simple query with another statement that Proxy needs to encrypt or decrypt, as passing the statement through sends the whole query to the database as written.
  Send the statement in a query of its own.

A statement that is passed through is sent to the database as written, so a plaintext value that it compares with an encrypted column, as in `WHERE email = 'a@example.com'`, reaches the database.

In our experience, most production systems have a relatively small number of columns that require protection.
As SQL is large and complex, instead of blocking statements with type check errors that are false negatives, the default behaviour of Proxy is to allow the statement.

However, this does mean it is possible that a statement that references encrypted columns cannot be type-checked, and it will be passed through to the database.
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


## COPY into encrypted column <a id='mapping-copy-into-encrypted-column'></a>

A `COPY ... FROM` statement would load data into an encrypted column.
CipherStash Proxy does not encrypt `COPY` data, so it refuses the statement rather than send plaintext to the encrypted column.

### Error message

```
COPY into encrypted column 'encrypted_email' of table 'users' is not supported, as Proxy does not encrypt COPY data.
```

### Notes

A `COPY` with no column list loads every column of the table, so it is refused when the table has any encrypted column.
A `COPY ... FROM` into a table with no encrypted columns, or into only the plaintext columns of a table, is forwarded to PostgreSQL with its data.
A `COPY ... TO` of a table or a query that reads is forwarded, and returns the encrypted columns as ciphertext, as Proxy does not decrypt `COPY` data.
Proxy checks a `COPY` with the SQL parser, so it refuses as invalid SQL the few `COPY` options the parser does not support, such as `WHERE`, `HEADER MATCH` and `ON_ERROR`, even with no encryption configured.

### How to fix

Load the encrypted columns with `INSERT` statements through CipherStash Proxy, which encrypts the values.
Use `COPY` only for columns that are not encrypted.


<!-- ---------------------------------------------------------------------------------------------------- -->


## COPY into unknown table <a id='mapping-copy-into-unknown-table'></a>

A `COPY ... FROM` statement would load data into a table or column that is not in the database schema CipherStash Proxy has loaded.
Proxy cannot tell whether the data would go to an encrypted column, so it refuses the statement.

### Error message

```
COPY into table 'users' is not supported, as Proxy cannot tell which of its columns are encrypted.
```

### Notes

Proxy loads the tables of every schema by name, without the schema.
A table name used in more than one schema is refused, as Proxy cannot tell which table is meant.
A table created on the same connection, earlier or in the same query, is known by the name it was created with, so `CREATE TABLE public.staging` is known to `COPY public.staging` but not to `COPY staging`.
A `COPY` is refused if either a table created with that name or the loaded table of that name has an encrypted column it would fill, as Proxy cannot tell whether PostgreSQL created the table, such as after `CREATE TABLE IF NOT EXISTS` or a rollback.
So a `COPY` into a temporary table named like an encrypted table is refused.
A `COPY` with no column list into a table whose columns Proxy does not know, such as one created with `CREATE TABLE ... AS`, is refused.
Any `COPY` into a table created with `CREATE TABLE ... (LIKE ...)` is refused, as Proxy does not know which columns it copies.
When no columns are encrypted, Proxy does not check the table of a `COPY`.

### How to fix

Check that the table and columns exist.
If the table was created on another connection, retry once Proxy has reloaded the database schema.
If the table name is used in more than one schema, rename one of the tables.


<!-- ---------------------------------------------------------------------------------------------------- -->


## COPY of a query that writes <a id='mapping-copy-of-query-that-writes'></a>

A `COPY (...) TO` statement has a query that writes, such as `INSERT ... RETURNING`, `UPDATE ... RETURNING` or `DELETE ... RETURNING`, or a `WITH` query that does.
CipherStash Proxy does not encrypt the query of a `COPY`, so it refuses the statement rather than send its values to PostgreSQL as plaintext.

### Error message

```
COPY of a query that writes is not supported, as Proxy does not encrypt the query of a COPY.
```

### Notes

When no columns are encrypted, Proxy forwards the `COPY`.

### How to fix

Run the statement on its own, without `COPY`, so Proxy encrypts it.


<!-- ---------------------------------------------------------------------------------------------------- -->


## COPY with a statement that uses an encrypted column <a id='mapping-copy-with-encrypted-statement'></a>

A simple query contains a `COPY` together with a statement that CipherStash Proxy encrypts.
Proxy rewrites such a query to encrypt it, and cannot keep the `COPY` in the rewritten query.

### Error message

```
COPY must be sent in a query of its own when the query has a statement that uses an encrypted column.
```

### How to fix

Send the `COPY` in a query of its own.


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


## Prepared settings changed <a id='mapping-prepared-settings-changed'></a>

The prepared statement or bound values no longer match the connection's CipherStash settings.
This happens when `CIPHERSTASH.UNSAFE_DISABLE_MAPPING` changes after a statement Proxy maps was prepared or bound, or when the keyset changes and the statement holds values encrypted under the previous keyset.
The change may come from a `SET`, a `RESET`, or a transaction rollback that undid the settings the statement was prepared under.
Proxy refuses to run the statement to avoid forwarding plaintext or values encrypted under the wrong keyset.

### Error message

```
The prepared statement or bound values no longer match the connection's CipherStash settings. Close the statement, then prepare and bind it again.
```

### How to fix

The error uses SQLSTATE `26000`, so drivers such as PostgreSQL JDBC can invalidate their cached statements.

Select the intended keyset and mapping mode, close the old statement, then prepare and bind it again.
Prefer bound parameters over encrypted literals when a prepared statement must be reused across keysets.



<!-- ---------------------------------------------------------------------------------------------------- -->


# Encrypt errors


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

The column has an encrypted type (PostgreSQL `eql_v2_encrypted` type ) with no encryption configuration.

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

The table has one or more encrypted columns (PostgreSQL `eql_v2_encrypted` type ) with no encryption configuration.

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
