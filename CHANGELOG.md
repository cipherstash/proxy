# Changelog

All notable changes to CipherStash Proxy will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/).

## [Unreleased]

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

[Unreleased]: https://github.com/cipherstash/proxy/compare/v2.1.20...HEAD
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
