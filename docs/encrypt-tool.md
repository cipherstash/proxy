# `encrypt` tool guide

CipherStash Proxy includes an `encrypt` tool – a CLI application to encrypt existing data, or to apply index changes after changes to the encryption configuration of a protected database.

## Table of contents

- [Using the `encrypt` tool](#using-the-encrypt-tool)
- [How the `encrypt` tool works](#how-the-encrypt-tool-works)
- [Configuring the `encrypt` tool](#configuring-the-encrypt-tool)
- [Example `encrypt` tool usage](#example-encrypt-tool-usage)

## Using the `encrypt` tool

Encrypt the `source` column data in `table` into the specified encrypted `target` column.
The `encrypt` tool connects to CipherStash Proxy using the `cipherstash.toml` configuration or `ENV` variables.

```
cipherstash-proxy encrypt [OPTIONS] --table <TABLE>  --columns <SOURCE_COLUMN=TARGET_COLUMN>...
```

## How the `encrypt` tool works

At a high-level, the process for encrypting a column in the database is:

1. Add a new encrypted destination column with the appropriate encryption configuration.
2. Using CipherStash Proxy to process:
  1. Select from the original plaintext column.
  2. Update the encrpted column to set the plaintext value.
3. Drop the original plaintext column.
4. Rename the encrypted column to the original plaintext column name.

The CipherStash Proxy `encrypt` tool automates the data process to encrypt one or more columns in a table.
Updates are executed in batches of 100 records (and the `batch_size` is configurable).
The process is idempotent and can be run repeatedly.

## Configuring the `encrypt` tool

The CipherStash Proxy `encrypt` tool reuses the CipherStash Proxy configuration for the Proxy connection details.
This configuration includes database server host, port, username, password, and database name.
The following table lists the available options.

| Option                  | Description                                                    | Default         |
| ----------------------- | -------------------------------------------------------------- | --------------- |
| `-t`, `--table`         | Specifies the table to migrate                                 | None (Required) |
| `-c`, `--columns`       | List of columns to migrate (space-delimited key=value pairs)   | None (Required) |
| `-k`, `--primary-key`   | List of primary key columns (space-delimited)                  | `id`            |
| `-b`, `--batch-size`    | Number of records to process at once                           | `100`           |
| `-d`, `--dry-run`       | Runs without updating. Loads data but does not perform updates | None (Optional) |
| `-v`, `--verbose`       | Turn on additional logging output                              | None (Optional) |
| `-h`, `--help`          | Displays this help message                                     | -               |

## Example `encrypt` tool usage

Given a running instance of CipherStash Proxy and a `users` table with:
 - `id` – a primary key column
 - `email` – a source plaintext column
 - `encrypted_email` – a destination column configured to be encrypted text.

Encrypt `email` into `encrypted_email`:

```bash
cipherstash-proxy encrypt --table users --columns email=encrypted_email
```

Specify the primary key column:

```bash
cipherstash-proxy encrypt --table users --columns email=encrypted_email --primary-key user_id
```

Specify multiple primary key columns (compound primary key):

```bash
cipherstash-proxy encrypt --table users --columns email=encrypted_email --primary-key user_id tenant_id
```

---

### Didn't find what you wanted?

[Click here to let us know what was missing from our docs.](https://github.com/cipherstash/proxy/issues/new?template=docs-feedback.yml&title=[Docs:]%20Feedback%20on%20encrypt-tool.md)
