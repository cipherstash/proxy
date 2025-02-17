


# Development Guide


## Logging

- Use structured logging
- Use te appropriate targets
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

