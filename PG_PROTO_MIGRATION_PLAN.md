# pg-proto Migration

## Summary

- Create `/Users/jamessadler/cipherstash/proxy-pg-proto` from current `main` (`15b7f996`) on branch `refactor/pg-proto`.
- Save this migration plan as `PG_PROTO_MIGRATION_PLAN.md` in that worktree’s repository root.
- Limit the initial deliverable to the worktree, branch, and plan document; implementation follows separately.
- Target a full migration to published [`pg-proto` 0.1.0](https://crates.io/crates/pg-proto/0.1.0), covering codecs, startup/authentication, and runtime protocol-state validation.

## Implementation Changes

- Replace handwritten framing, startup packet parsing, message codes, and message serialization with direction-specific `pg-proto` frontend/backend codecs.
- Convert CipherStash-specific behavior into adapters over `pg-proto` messages:
  - Preserve Parse/Query SQL rewriting and parameter OID mapping.
  - Preserve Bind format-code semantics, nulls, parameter reshaping, and encryption.
  - Preserve ParameterDescription, RowDescription, and DataRow rewriting and batched decryption.
  - Retain diagnostic-response factories while emitting `pg-proto` response types.
- Use `pg-proto` pre-startup and authentication APIs for SSL negotiation, startup, cancellation, client-facing MD5 authentication, and upstream cleartext/MD5/SCRAM authentication. Continue using existing TLS configuration and certificate policy.
- Pair downstream server-role and upstream client-role runtime FSMs through `Intermediary`. Advance both sides for forwarded messages and only the affected side for locally intercepted or synthesised messages.
- Preserve concurrent client-to-server and server-to-client processing, connection timeouts, response ordering, metrics, logging, schema reloads, and row buffering.
- Track protocol state even when encryption mapping is disabled. Preserve one-to-one cancellation forwarding; do not introduce pooling or cancellation-key translation.
- Reject unknown message tags as protocol errors, matching `pg-proto`’s fail-closed behavior.
- Remove obsolete handwritten protocol modules and direct low-level dependencies once unused. Preserve the public configuration and CLI surfaces; retain existing `ProtocolError` variants for source compatibility even where `pg-proto` supersedes them.

## Test Plan

- Port existing message round-trip and rewrite tests to `pg-proto` message fixtures.
- Add coverage for partial/oversized frames, malformed messages, unknown-tag rejection, SSL/TLS startup, cancellation, and all supported authentication modes.
- Exercise simple queries and extended Parse/Bind/Describe/Execute/Close/Sync pipelines, including pipelining and error draining through Sync.
- Verify text/binary formats, nulls, reshaped parameters, prepared statements, portals, COPY messages, asynchronous backend messages, and buffered DataRow decryption.
- Run formatting, clippy, proxy unit tests, and TCP/TLS integration suites. Unset `CS_PROMETHEUS__ENABLED` for the baseline unit suite; its current environment value causes the otherwise unrelated Prometheus test to fail.

## Assumptions

- The existing untracked `.claude/worktrees/` directory remains untouched.
- The worktree path and branch are currently available.
- No compatibility feature flag or dual protocol implementation is required.
- The plan document is left as an uncommitted worktree change unless a commit is requested separately.
