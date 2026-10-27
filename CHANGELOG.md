# Changelog

All notable changes to CipherStash Proxy will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

### Fixed

- **Memory growth on long-lived connections**: Proxy now releases the prepared-statement, portal and statement-metrics state it keeps for each client connection once PostgreSQL has finished with it. Previously some common client patterns left state behind until the connection closed, so with a connection pool that keeps connections open, memory grew steadily until Proxy was restarted. The affected patterns were statements prepared and described but never executed, prepared statements removed with SQL `DEALLOCATE`, `DEALLOCATE ALL` or `DISCARD ALL` (used by psycopg 3, Rails and connection-pool reset queries, whether sent as a simple query or through the extended protocol), uniquely named portals, pipelined batches that fail part-way, statements rejected by Proxy, and connections that use `COPY ... FROM STDIN`. No configuration change is required.

- **Connection settings after a pool reset**: `DISCARD ALL` and `RESET ALL` now return Proxy's per-connection settings to their startup values once PostgreSQL confirms the command. The keyset chosen with `SET CIPHERSTASH.KEYSET_ID` or `KEYSET_NAME` is cleared, so Proxy uses `default_keyset_id` (or requires a keyset again when there is none), and `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true` is undone. `RESET CIPHERSTASH.KEYSET_ID` or `RESET CIPHERSTASH.KEYSET_NAME` clears the keyset, `RESET CIPHERSTASH.UNSAFE_DISABLE_MAPPING` turns mapping back on, and the equivalent `SET CIPHERSTASH.<setting> TO DEFAULT` does the same. Previously a connection pool that reset a connection between clients handed the next client the previous client's keyset, or left mapping disabled so the next client's queries were sent unencrypted; `SET ... TO DEFAULT` left the setting unchanged or, for `KEYSET_NAME`, selected a keyset named "DEFAULT". A reset that PostgreSQL rejects, for example inside a failed transaction, changes nothing. A `SET CIPHERSTASH.<setting>` prepared as a named statement now takes effect each time it runs, so running it again after a reset selects the keyset again, as it does in PostgreSQL. A `SET CIPHERSTASH.KEYSET_ID` or `KEYSET_NAME` sent while mapping is disabled is now kept when no `default_keyset_id` is configured, so the keyset is in place when mapping is turned back on. As a result, a `SET CIPHERSTASH.KEYSET_ID` that is not a valid UUID is now refused while mapping is disabled, as it is when mapping is enabled; previously it was forwarded to PostgreSQL. A prepared `SET CIPHERSTASH.<setting>` now takes effect only when it is executed, not when it is prepared. `RESET` statements, which Proxy previously rejected as invalid SQL, are now forwarded to PostgreSQL when a query contains only `RESET` statements.

- **Connection settings in transactions**: `SET CIPHERSTASH.KEYSET_ID`, `KEYSET_NAME` and `UNSAFE_DISABLE_MAPPING` now follow PostgreSQL's rules for settings in transactions. A change, including a `RESET`, `SET ... TO DEFAULT` or `DISCARD ALL`, takes effect once PostgreSQL confirms it, is kept on `COMMIT`, and is undone on `ROLLBACK` or `ROLLBACK TO SAVEPOINT`, or when the transaction fails, including the implicit transaction of a simple query or extended-protocol batch. `SET LOCAL` now lasts only until the end of the transaction. Previously Proxy kept these settings outside PostgreSQL's transactions: a rolled-back reset left Proxy with no keyset, so later writes were encrypted under `default_keyset_id` instead of the selected keyset; a rolled-back `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING = true` left mapping disabled; and `SET LOCAL` was treated as a plain `SET`. Setting names are now matched as PostgreSQL matches them, ignoring case and quoting, and `UNSAFE_DISABLE_MAPPING` accepts every PostgreSQL boolean spelling. A `SET CIPHERSTASH.*` that Proxy cannot honour, such as an unknown setting name or a value it cannot interpret, is now refused with an error instead of being ignored. A simple query that Proxy rewrites now forwards every statement in it; previously statements Proxy did not map, such as a `SET`, could be dropped from a query that also encrypted values. A prepared statement is now refused with SQLSTATE `26000` if mapping has been turned on or off since it was prepared or bound, or if the keyset has changed and it holds values encrypted under the previous keyset, for example after a rollback undid the change it was prepared under; prepare it again to run it under the new settings. A statement or message that Proxy refuses now fails the batch, and an open transaction, in PostgreSQL as well, as an error from PostgreSQL would. Previously Proxy answered a refused extended-protocol batch itself, so statements earlier in the batch could still commit and the client's transaction carried on as if nothing had failed. A statement pipelined after a query or batch that may change these settings now waits for PostgreSQL to answer it, so it is mapped under the settings PostgreSQL confirmed. Previously Proxy assumed the change succeeded, so a `SET` that failed, or a `SET LOCAL` that ended with its implicit transaction, still applied to the statements sent after it.

- **`COPY ... FROM STDIN`**: Proxy now forwards a `COPY ... FROM STDIN` into a table with no encrypted columns, with its copy data, so `psql`'s `\copy` and driver `COPY` APIs can bulk-load plaintext tables through Proxy. Previously Proxy refused the statement as invalid SQL unless it ended with `;`, with or without an encryption configuration. A `COPY ... FROM` into an encrypted column is now refused with an error that names the column, as Proxy does not encrypt `COPY` data; previously a `COPY` from a file or program was forwarded with its plaintext. When encrypted columns are configured, a `COPY ... FROM` into a table or column that Proxy has not loaded, or into a table name that more than one schema has, is refused, as Proxy cannot tell whether it is encrypted. Statements after a `COPY ... FROM STDIN;` in the same simple query are now checked and encrypted like any other; previously Proxy read them as copy data and forwarded them unchanged, so a plaintext `INSERT` into an encrypted column could reach PostgreSQL. A simple query that has a `COPY` and a statement Proxy encrypts is refused, and the `COPY` must be sent in a query of its own. A `COPY (INSERT ... RETURNING ...) TO STDOUT`, or a `COPY` of another query that writes, is refused when encrypted columns are configured; previously Proxy forwarded its values as plaintext. Proxy now parses every `COPY`, so it refuses as invalid SQL the few `COPY` options its SQL parser does not support, such as `WHERE`, `HEADER MATCH` and `ON_ERROR`.

- **Plaintext forwarded beside a statement Proxy cannot type check**: When encrypted columns are configured, Proxy now refuses a write that it cannot type check, such as an `INSERT` into a table it has not loaded, instead of forwarding it to PostgreSQL unchanged. Such a write is still forwarded when Proxy knows that every table it writes to has no encrypted columns. Proxy also refuses a simple query that has a statement it cannot type check together with a statement it needs to encrypt or decrypt. Previously, with mapping errors off (the default), Proxy forwarded the whole query as written, so an `INSERT` into an encrypted column in the same query reached PostgreSQL with its plaintext values. A read that Proxy cannot type check is still forwarded unchanged when it is sent in a query of its own or only with statements that Proxy does not need to map. A statement that Proxy forwards unchanged is sent as written, so a plaintext value that it compares with an encrypted column still reaches the database.

### Security

- **Dependency updates**: Updated `rustls-webpki` to 0.103.14, `rand` to 0.9.3 and 0.10.2, `quinn-proto` to 0.11.15, `hickory-proto` and `hickory-resolver` to 0.26.3 (with `reqwest` 0.13.4), `diesel` to 2.3.12, and `postgres-protocol` to 0.6.12, picking up the fixes for published security advisories in those crates that Proxy 3.x already ships. No configuration change is required.

## [2.2.4] - 2026-06-18

### Fixed

- **ZeroKMS authentication failures ~15 minutes after startup (access keys)**: Fixed the root cause of access tokens never being renewed when authenticating with an access key. The token's lifetime was misread, so renewal never triggered and every encrypt/decrypt operation began failing (`ZeroKMS error: Request not authorized`, "Could not decrypt data") roughly 15 minutes — the token lifetime — after connecting, recovering only on restart. Tokens now renew correctly ahead of expiry. This resolves the remaining cases not addressed by the 2.2.3 fix.

## [2.2.3] - 2026-06-17

### Fixed

- **ZeroKMS authentication failures ~15 minutes after startup**: Fixed an issue in the access-key authentication path where, after an in-flight request was interrupted at the wrong moment (for example, a client disconnecting mid-query), access-token renewal could stall. This caused `ZeroKMS error: Request not authorized` on all encrypt/decrypt operations roughly 15 minutes (the access-token lifetime) after connecting — connections worked on startup and then began failing in lockstep.

## [2.2.2] - 2026-06-01

### Fixed

- **Passthrough mode memory leak**: Fixed a per-statement memory leak that occurred in passthrough mode (empty encrypt config), where per-statement queues were not drained. Long-running connections could grow unbounded and eventually OOM. ([#400](https://github.com/cipherstash/proxy/issues/400))

## [2.2.1] - 2026-05-14

### Added

- **OPE (Order-Preserving Encryption) index**: New `ope` index type alongside the existing `ore` for range and `ORDER BY` queries on encrypted columns. Drop-in alternative to `ore` — pick one per column. See the [encrypted indexes documentation](docs/how-to/index.md) for configuration.

## [2.2.0-alpha.1] - 2026-03-25

### Changed

- **Log target renamed**: `KEYSET` log target renamed to `ZEROKMS`. The environment variable `CS_LOG__KEYSET_LEVEL` is now `CS_LOG__ZEROKMS_LEVEL`.

### Removed

- **Log target removed**: `PROXY` log target and `CS_LOG__PROXY_LEVEL` environment variable have been removed.

### Added

- **Cipher cache miss metric**: New Prometheus counter `cipherstash_proxy_keyset_cipher_cache_miss_total` tracks cache misses requiring cipher initialization. This complements the `cipherstash_proxy_keyset_cipher_cache_hit_total` metric, and can be used to calculate cache hit/miss ratio.
- **Cipher init duration metric**: New Prometheus histogram `cipherstash_proxy_keyset_cipher_init_duration_seconds` tracks cipher initialization time including ZeroKMS network calls.
- **Encrypt/decrypt timing**: Debug logs for `encrypt_eql` and `decrypt_eql` now include `duration_ms`.
- **Cache eviction logging**: ScopedCipher cache eviction events are now logged under the `ZEROKMS` target.
- **Slow cipher init warning**: Cipher initialization taking longer than 1 second triggers a warning log.

## [2.1.22] - 2026-02-05

### Added

- **Configurable slow database response threshold**: The "Slow database response" log threshold is now configurable via `CS_LOG__SLOW_DB_RESPONSE_MIN_DURATION_MS` (default: 100ms). This controls per-message logging for individual slow reads from the PostgreSQL server.

## [2.1.21] - 2026-02-04

### Changed

- Updated `cipherstash-client` to v0.33.0. Adds `array_index_mode` configuration for STE-VEC indexes, which controls how arrays are indexed in JSONB data. Defaults to `all` (generating item, wildcard, and positional selectors), preserving backwards compatibility with existing configurations.

## [2.1.20] - 2026-01-29

### Added

- **Slow statement logging**: Enable with `CS_LOG__SLOW_STATEMENTS=true` to log detailed timing breakdowns when queries exceed a configurable threshold (default 2 seconds). Includes breakdown of parse, encrypt, server wait, and decrypt phases.
- **Prometheus slow statement counter**: New `cipherstash_proxy_slow_statements_total` metric increments when slow statements are detected.
- **Prometheus histogram labels**: Duration histograms now include `statement_type`, `protocol`, `mapped`, and `multi_statement` labels for granular performance analysis.
- **Term filters for STE-VEC indexes**: Support for `term_filters` configuration in `eql_v2.add_search_config()`, enabling case-insensitive JSONB queries with the `downcase` filter.

### Changed

- Updated `cipherstash-client` to v0.32.2.
- GitHub Actions jobs now timeout after 30 minutes.
- ARM64 builds migrated to Blacksmith runners.

## [2.1.9] - 2026-01-10

### Added

- Encryption sanity checks for improved error detection.
- Developer documentation updates.

### Changed

- Updated `cipherstash-client` to v0.31.1.

## [2.1.8] - 2025-12-15

### Changed

- Refactored EQL encryption logic.
- JSONB containment operator transformation improvements.
- Testing across multiple PostgreSQL versions.

## [2.1.7] - 2025-11-27

### Added

- Security documentation.

### Changed

- Improved ZeroKMS error handling.
- Database connection CLI arguments now optional.

## [2.1.6] - 2025-09-05

### Fixed

- Accurate cipher cache sizing.
- JSONB encrypted type protocol fixes.

### Changed

- Module restructuring.

## [2.1.5] - 2025-08-21

### Added

- `SET` command for `keyset_id` configuration.
- Configurable cipher caching using async Moka.

## [2.1.4] - 2025-08-08

### Changed

- Updated EQL to v2.1.8.

## [2.1.3] - 2025-08-01

### Added

- Helm chart support.
- JSONB operator integration tests.
- Comprehensive proxy/EQL showcase crate.

## [2.1.2] - 2025-07-16

### Fixed

- Common Table Expression (CTE) table resolution in EQL mapper.

## [2.1.1] - 2025-07-15

### Added

- JSON indexing for EQL v2.
- Prometheus metrics collection.
- Multiple integration test frameworks.

## [2.0.10] - 2025-06-26

### Added

- `SET` command to disable mapping.

## [2.0.9] - 2025-06-20

### Changed

- Upgraded container base image to Ubuntu 25.10.
- Updated sqltk dependency to v0.10.0.

## [2.0.8] - 2025-06-18

### Added

- Version string sent to ZeroKMS/CTS requests.

### Fixed

- Type-related issues in sqlparser.

### Changed

- Release workflow now triggers on release events.

## [2.0.7] - 2025-06-12

### Added

- Language-specific tests in integration suite.
- PostgreSQL custom and domain type identifier handling.

### Fixed

- Docker image build processes in GitHub Actions.

## [2.0.6] - 2025-06-09

### Added

- TLS and Docker configuration documentation.
- Expanded test coverage for order and group operations.

### Changed

- URL encoding for usernames in Docker entrypoint.
- Preference for CRN over workspace_id and region.

### Removed

- Order and group transformers.

## [2.0.5] - 2025-05-27

### Fixed

- Cache usage in release artifact building.

## [2.0.4] - 2025-05-26

### Added

- OIDC support.

### Fixed

- Special character handling in database configuration.
- "Insufficient data left in message" errors with null values.

## [2.0.3] - 2025-05-26

### Fixed

- Tests now ignore `CS_` environment variables during configuration validation.

### Changed

- Added environment debugging to AWS Marketplace release workflow.

## [2.0.2] - 2025-05-22

### Added

- Multi-platform Docker image builds.

### Changed

- Updated EQL to v2.0.1.

## [2.0.1] - 2025-05-21

### Added

- Encryption configuration validation.
- pgbench performance testing integration.
- ZeroKMS and CTS host configuration options.
- `GROUP BY` SQL transformations.
- EQL v2 decryption support.
- Enhanced column configuration verification.

### Fixed

- Connection termination messaging.

### Changed

- Upgraded to Rust 1.86.0 compatibility.
- Upgraded sqltk to v0.8.0.

## [2.0.0] - 2025-03-27

### Added

- Initial release of CipherStash Proxy.
- Transparent proxy for PostgreSQL with automatic encryption/decryption.
- Support for queries over encrypted values (equality, comparison, ordering).
- Docker container deployment.
- Integration with CipherStash ZeroKMS.
- Encrypt Query Language (EQL) for indexing and searching encrypted data.

[Unreleased]: https://github.com/cipherstash/proxy/compare/v2.2.4...HEAD
[2.2.4]: https://github.com/cipherstash/proxy/compare/v2.2.3...v2.2.4
[2.2.3]: https://github.com/cipherstash/proxy/compare/v2.2.2...v2.2.3
[2.2.2]: https://github.com/cipherstash/proxy/compare/v2.2.1...v2.2.2
[2.2.1]: https://github.com/cipherstash/proxy/compare/v2.2.0-alpha.1...v2.2.1
[2.2.0-alpha.1]: https://github.com/cipherstash/proxy/compare/v2.1.22...v2.2.0-alpha.1
[2.1.22]: https://github.com/cipherstash/proxy/releases/tag/v2.1.22
[2.1.21]: https://github.com/cipherstash/proxy/releases/tag/v2.1.21
[2.1.20]: https://github.com/cipherstash/proxy/releases/tag/v2.1.20
[2.1.9]: https://github.com/cipherstash/proxy/releases/tag/v2.1.9
[2.1.8]: https://github.com/cipherstash/proxy/releases/tag/v2.1.8
[2.1.7]: https://github.com/cipherstash/proxy/releases/tag/v2.1.7
[2.1.6]: https://github.com/cipherstash/proxy/releases/tag/v2.1.6
[2.1.5]: https://github.com/cipherstash/proxy/releases/tag/v2.1.5
[2.1.4]: https://github.com/cipherstash/proxy/releases/tag/v2.1.4
[2.1.3]: https://github.com/cipherstash/proxy/releases/tag/v2.1.3
[2.1.2]: https://github.com/cipherstash/proxy/releases/tag/v2.1.2
[2.1.1]: https://github.com/cipherstash/proxy/releases/tag/v2.1.1
[2.0.10]: https://github.com/cipherstash/proxy/releases/tag/v2.0.10
[2.0.9]: https://github.com/cipherstash/proxy/releases/tag/v2.0.9
[2.0.8]: https://github.com/cipherstash/proxy/releases/tag/v2.0.8
[2.0.7]: https://github.com/cipherstash/proxy/releases/tag/v2.0.7
[2.0.6]: https://github.com/cipherstash/proxy/releases/tag/v2.0.6
[2.0.5]: https://github.com/cipherstash/proxy/releases/tag/v2.0.5
[2.0.4]: https://github.com/cipherstash/proxy/releases/tag/v2.0.4
[2.0.3]: https://github.com/cipherstash/proxy/releases/tag/v2.0.3
[2.0.2]: https://github.com/cipherstash/proxy/releases/tag/v2.0.2
[2.0.1]: https://github.com/cipherstash/proxy/releases/tag/v2.0.1
[2.0.0]: https://github.com/cipherstash/proxy/releases/tag/v2.0.0
