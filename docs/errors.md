

# Errors

## Table of Contents

- Mapping Errors:
  - [Invalid parameter](#mapping-invalid-parameter)
  - [Unsupported parameter type](#mapping-unsupported-parameter-type)
- Encrypt Errors:
  - [Column could not be encrypted](#encrypt-column-could-not-be-encrypted)
  - [Plaintext could not be encoded](#encrypt-plaintext-could-not-be-encoded)
  - [Unknown column](#encrypt-unknown-column)
  - [Unknown table](#encrypt-unknown-table)
  - [Unknown index term](#encrypt-unknown-index-term)

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

Check that CipherStash ZeroKMS is available.
See [CipherStash Status](https://status.cipherstash.com/).

Check that CipherStash Proxy has network access to ZeroKMS in the appropriate region.
See [TODO: Link to ZeroKMS Doc](https://).

Check that the encrypted configuration `cast` matches the expected type.
See [TODO: Link to config](https://).



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

Check the encrypted configuration has the correct type.

Check that the configuration has not changed.

See [EQL](https://github.com/cipherstash/encrypt-query-language).


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

Define the encrypted configuration using [EQL](https://github.com/cipherstash/encrypt-query-language).

<!-- TODO: link to doc -->

Adding `users.email` as an encrypted column:

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

Define the encrypted configuration using [EQL](https://github.com/cipherstash/encrypt-query-language).

<!-- TODO: link to doc -->

Adding `users.email` as an encrypted column:

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

Check the Encrypt configuration for the column.

Define the encrypted configuration using [EQL](https://github.com/cipherstash/encrypt-query-language).




