


# Development Guide


## Logging

- Use structured logging
- Use the appropriate targets
- Include the `client_id` where appropriate

Debug logging is very verbose, and targets allow configuration of granular log levels.

A `target` is a string value that is added to the standard tracing macro calls (`debug!, error!, etc`).
Log levels can be configured for each `target` individually.

A number of targets are already defined in `log.rs`.

The targets are aligned with the different components and contexts (`PROTOCOL, AUTHENTICATION, MAPPER, etc`)

There is a general `DEVELOPMENT` target for logs that don't quite fit into a specific category.


### Available targets

```
Target          | ENV
--------------- | -------------------------------------
DEVELOPMENT     | CS_LOG__DEVELOPMENT_LEVEL
AUTHENTICATION  | CS_LOG__AUTHENTICATION_LEVEL
CONTEXT         | CS_LOG__CONTEXT_LEVEL
ENCRYPT         | CS_LOG__ENCRYPT_LEVEL
KEYSET          | CS_LOG__KEYSET_LEVEL
PROTOCOL        | CS_LOG__PROTOCOL_LEVEL
MAPPER          | CS_LOG__MAPPER_LEVEL
SCHEMA          | CS_LOG__SCHEMA_LEVEL
```


### Example

The default log level for the proxy is `info`.

An `env` variable can be used to configure the logging level.

Configure `debug` for the `MAPPER` target:

```shell
CS_LOG__MAPPER_LEVEL = "debug"
```

Log `debug` output for the `MAPPER` target:

```rust
    debug!(
        target: MAPPER,
        client_id = self.context.client_id,
        identifier = ?identifier
    );
```

## Style Guide

### Testing

#### Use `unwrap()` instead of `expect()` unless providing context
When working with `Result` and `Option` in Rust tests, prefer `unwrap()` over `expect()` unless the error message provides meaningful context.
While both are functionally equivalent, `expect()` can introduce unnecessary noise if its message is generic.
If additional context is necessary, use `expect()` with a clear explanation of why the value should be `Ok` or `Some`.

Reference: [Rust documentation on `expect`](https://doc.rust-lang.org/std/result/enum.Result.html#method.expect)

#### Prefer `assert_eq!` over `assert!` for equality checks
Use `assert_eq!` instead of `assert!` when testing equality in Rust.
While both achieve the same result, `assert_eq!` provides clearer failure messages by displaying the expected and actual values, making debugging easier.
