


# Development Guide




## Logging

- Use structured logging
- Use targets
- Include the `client_id` where appropriate

Debug logging is very verbose, and targets allow configuration of granular log levels.

A `target` is a string value that is added to the standard tracing macro calls (`debug!, error!, etc`).
Log levels can be configured for eachj `target` individually.

A number of targets are already defined in `log.rs`.

The targets are aligned with the different components and contexts (`PROTOCOL, AUTHENTICATION, MAPPER, etc`)

There is a general `DEVELOPMENT` target for logs that don't quite fit into a specific category.


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