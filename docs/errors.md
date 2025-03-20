

# Errors

## Table of Contents

- Authentication Errors:
  - [Database](#authentication-failed-database)
  - [Client](#authentication-failed-client)

- Mapping Errors:
  - [Invalid parameter](#mapping-invalid-parameter)
  - [Invalid SQL statement](#mapping-invalid-sql-statement)
  - [Unsupported parameter type](#mapping-unsupported-parameter-type)
  - [Statement could not be type checked](#mapping-statement-could-not-be-type-checked)
  - [Internal Error](#mapping-internal-error)

- Encrypt Errors:
  - [Column could not be encrypted](#encrypt-column-could-not-be-encrypted)
  - [Plaintext could not be encoded](#encrypt-plaintext-could-not-be-encoded)
  - [Unknown column](#encrypt-unknown-column)
  - [Unknown table](#encrypt-unknown-table)
  - [Unknown index term](#encrypt-unknown-index-term)

<!-- ---------------------------------------------------------------------------------------------------- -->

<!-- ---------------------------------------------------------------------------------------------------- -->

# Authentication Errors


## Database <a id='authentication-failed-database'></a>

Authentication failed when connecting to the database.


### Error message

```
Database authentication failed: check username and password
```

### How to Fix

Check the configured username and password are correct and can connect to the database.

Check the database is using a supported authentication method.

CipherStash Proxy supportsseveral PostgreSQL password authentication methods:

 - password
 - md5
 - scram-sha-256

 See [PostgreSQL password authentication](https://www.postgresql.org/docs/17/auth-password.html)


<!-- ---------------------------------------------------------------------------------------------------- -->



## Client <a id='authentication-failed-client'></a>

Authentication failed when connecting a client to the proxy.


### Error message

```
Client authentication failed: check username and password
```

### How to Fix

Check the configured username and password are correct.



<!-- ---------------------------------------------------------------------------------------------------- -->



<!-- ---------------------------------------------------------------------------------------------------- -->

# Mapping Errors


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


### How to Fix

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


### How to Fix

Check the sql is a valid PostgreSQL SQL statement.


<!-- ---------------------------------------------------------------------------------------------------- -->


## Unsupported Parameter type <a id='mapping-unsupported-parameter-type'></a>

The parameter type is not supported.


### Error message

```
Encryption of PostgreSQL {name} (OID {oid}) types is not currently supported.
```

### How to Fix

Check the supported types for encrypted columns.

<!-- TODO: link to doc -->



<!-- ---------------------------------------------------------------------------------------------------- -->


## Statement could not be type checked <a id='mapping-statement-could-not-be-type-checked'></a>

An error occurred when attempting to type check the SQL statement.

### Error message

```
Statement could not be type checked: '{type-check-erro-message}'
```

### Notes

CipherStash Proxy checks SQL statements against the database schema to transparently encrypt and decrypt data.

The behaviour of Proxy depends on the `mapping_errors_enabled` configuration.

When `mapping_errors_enabled` is `false` (the default), then type check errors are logged, and the statement is passed through to the database.

When `mapping_errors_enabled` is `true`, then type check errors are raised, and statement execution halts.

In our experience, most production systems have a relatively small number of columns that require protection.
As SQL is large and complex, instead of blocking statements with type check errors that are false negatives, the default behaviour of Proxy is to allow the statement.

However, this does mean it is possible that a statement referencing encrypted columns cannot be type-checked and is passed through to the database.
When a statement is passed through to the database, the database's column constraints (provided by EQL) will catch the statement, and return a PostgreSQL error.

Example constraint error:
```sql
ERROR:  Encrypted column missing version (v) field: 34234
CONTEXT:  PL/pgSQL function _cs_encrypted_check_v(jsonb) line 6 at RAISE
SQL function "cs_check_encrypted_v1" statement 1
```

### How to Fix

In most cases, this error will occur if the statement contains invalid or unsupported syntax.

Check that the installed version of CipherStash Proxy, and update to the latest version if possible.

If the error persists, please contact CipherStash [support](https://cipherstash.com/support).



<!-- ---------------------------------------------------------------------------------------------------- -->


## Internal Mapper error <a id='mapping-internal-error'></a>

An internal error occurred when attempting to type check or transform a SQL statement.
This could be due to an internal invariant failure or because of a specific fragment of unsupported SQL syntax.

### Error message

```
Statement encountered an internal error. This may be a bug in the statement mapping module of CipherStash Proxy.
```

### How to Fix

Check that the installed version of CipherStash Proxy, and update to the latest version if possible.

If the error persists, please contact CipherStash [support](https://cipherstash.com/support).



<!-- ---------------------------------------------------------------------------------------------------- -->


# Encrypt Errors


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
1. Check that CipherStash Proxy has network access to ZeroKMS in the appropriate region.
<!-- TODO: Link to ZeroKMS Doc -->
1. Check that the encrypted configuration `cast` matches the expected type.
<!-- TODO: Link to config -->



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


### How to Fix

1. Check the encrypted configuration has the correct type.
1. Check that the configuration has not changed.
1. Check [EQL](https://github.com/cipherstash/encrypt-query-language).

<!-- ---------------------------------------------------------------------------------------------------- -->

## Unknown Column <a id='encrypt-unknown-column'></a>

The column has an encrypted type (PostgreSQL `cs_encrypted_v1` type ) with no encryption configuration.

Without the configuration, Cipherstash Proxy does not know how to encrypt the column.
Any data is unprotected and unencrypted.


### Error message

```
Column 'column_name' in table 'table_name' has no Encrypt configuration
```


### How to Fix

1. Define the encrypted configuration using [EQL](https://github.com/cipherstash/encrypt-query-language).
   <!-- TODO: link to doc -->
1. Add `users.email` as an encrypted column:
   ```sql
   SELECT cs_add_column_v1('users', 'email');
   ```

<!-- ---------------------------------------------------------------------------------------------------- -->


## Unknown Table <a id='encrypt-unknown-table'></a>

The table has one or more encrypted columns (PostgreSQL `cs_encrypted_v1` type ) with no encryption configuration.

Without the configuration, Cipherstash Proxy does not know how to encrypt the column.
Any data is unprotected and unencrypted.


### Error message

```
Table 'table_name' has no Encrypt configuration
```

### How to Fix

1. Define the encrypted configuration using [EQL](https://github.com/cipherstash/encrypt-query-language).
   <!-- TODO: link to doc -->
1. Add `users.email` as an encrypted column:
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


### How to Fix

1. Check the Encrypt configuration for the column.
1. Define the encrypted configuration using [EQL](https://github.com/cipherstash/encrypt-query-language).


