---
status: accepted
issue: BUG-308
---

# Transaction-aware schema middleware

## Context

Proxy must map a statement using the schema PostgreSQL makes visible to that statement. DDL is
transactional: its effects become visible inside the transaction after successful execution,
but other connections cannot observe them until the outermost transaction commits.

The existing design marks schema change while parsing SQL and reloads through a separate database
connection. In the extended protocol, a client's preparation `Sync` can consume that marker before
the DDL is executed. Even moving the reload to every `ReadyForQuery` is insufficient: a
`ReadyForQuery(T)` occurs inside an explicit transaction, where the loader connection still cannot
see uncommitted DDL. Schema and column encryption configuration are also loaded and published
separately, while existing connections retain snapshots taken when their contexts were created.

These behaviours can make later connections use stale mapping or encryption metadata. They also
make connection-local behaviour depend on speculative DDL inferred at `Parse`, even if execution
later fails.

## Decision

Introduce standalone schema middleware as the sole owner of the transactional schema lifecycle.
Frontend and Backend report protocol events to it; neither manipulates schema-change flags or
reload managers directly.

### State model

- A **committed schema snapshot** is immutable and monotonically versioned. It contains database
  structure and encryption metadata derived from EQL domain types as one atomic value.
- A transaction pins the current committed snapshot. Existing idle connections adopt the latest
  snapshot before starting their next transaction.
- A **transaction schema overlay** contains only effects confirmed by successful DDL execution.
  The connection's **effective schema** is its pinned snapshot plus this overlay.
- Savepoints checkpoint the overlay. `ROLLBACK TO SAVEPOINT` restores its checkpoint, full rollback
  discards it, and release preserves its effects in the enclosing transaction.
- Parsed or prepared DDL records intent with the prepared statement. Each successful execution
  applies its effect; `Parse` alone never changes schema state.
- If a successful DDL cannot be modelled accurately, later schema-dependent statements in that
  transaction fail closed.

### Protocol ordering

After a DDL `Execute` is forwarded, protocol-control messages required to complete that execution
continue to flow, but later schema-dependent operations wait until its success or failure is known.
This avoids both speculative mapping and a deadlock in the extended-protocol prepare flow.

A simple-query message containing DDL followed by a schema-dependent statement fails closed for
the initial implementation. Supporting that case requires preserving PostgreSQL's response
semantics while introducing an execution boundary and will be tracked separately.

### Publication

When the outermost transaction containing successful DDL commits, one reload coordinator reads the
authoritative PostgreSQL catalog. Concurrent publication requests are coalesced, and generation
ordering prevents an older reload from replacing newer state. The coordinator atomically publishes
the combined schema and encryption snapshot before Proxy forwards `ReadyForQuery(I)`.

Proxy does not merge its inferred overlay into shared state. PostgreSQL remains authoritative for
cascades, conditional DDL, server-side effects, and the final outcome of the transaction.

If publication fails after PostgreSQL has committed, Proxy retains the dirty publication for retry,
does not forward successful readiness, and closes the affected client connection. The database
commit cannot be undone, but Proxy must not imply that stale encryption metadata is safe to use.

## Consequences

- DDL becomes visible to later statements on the same connection immediately after successful
  execution, including within an explicit transaction.
- Other connections observe DDL only after commit and successful publication.
- Every transaction maps against a stable schema and encryption-policy generation.
- Frontend and Backend become protocol adapters around a testable schema state machine.
- Extended-protocol pipelining requires bounded deferral after DDL execution.
- Availability is intentionally sacrificed when committed schema state cannot be published safely.
- Schema and encryption managers can no longer publish independent observable states.

## Verification

State-machine tests cover successful execution, execution failure, explicit commit, full rollback,
savepoint rollback, generation ordering, deferral, unmodelled DDL, and reload failure. Database-backed
tests cover extended-protocol autocommit, explicit transactions, an already-open second connection,
pipelining, direct ciphertext verification, and failure before readiness. The existing simple-query
behaviour remains covered, with dependent post-DDL batches asserted to fail closed.
