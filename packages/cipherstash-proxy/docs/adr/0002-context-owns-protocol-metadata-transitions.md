---
status: accepted
---

# Context owns CipherStash protocol metadata transitions

`Context` is the connection seam for CipherStash metadata associated with PostgreSQL Statements,
Portals, operations, and statement metrics scopes. Frontend and Backend remain protocol adapters:
they interpret wire messages, while Context applies each correlated metadata transition atomically
and returns the knowledge or effects the adapter needs. pg-proto continues to own protocol ordering
and backpressure, and Schema middleware continues to own transactional schema state under ADR-0001.

The correlated protocol state is kept in one internal state model rather than independently locked
maps. Context never holds that state lock across asynchronous work, metrics emission, or Schema
middleware calls; a transition first changes protocol state and returns explicit effects, then
Context applies those effects. Passthrough and encrypted traffic use the same lifecycle seam, and
inaccessible, stale, or inconsistent state fails the connection closed rather than silently omitting
a transition.

Statement metrics scopes belong to execution occurrences, not prepared Statements. A suspended and
resumed execution retains one scope until completion or failure; distinct Portals and repeated
executions receive distinct scopes, while Parse timing remains knowledge of the prepared Statement
that may be attributed to each execution observation. Existing Prometheus metric names remain stable.

Context exposes focused semantic lifecycle methods rather than a generic event dispatcher or
operation handles. Parse and Bind have explicit local preparation phases around asynchronous
mapping; finishing local preparation does not acknowledge PostgreSQL execution or retire its
rollback journal. Journal mutations include their complete metadata snapshots when created.
The underlying settings state machine and rollback algorithm remain private implementation.

Backend decrypts buffered rows before reporting the following PostgreSQL outcome, preserving
the effective connection settings those rows belong to. A local decryption failure ends local
execution observation without inventing a PostgreSQL failure; later suppressed replies still
report their actual settings and schema outcomes. Context owns readiness cleanup, awaited schema
publication, and schema readiness in that order, before Backend forwards readiness. Tests exercise
these lifecycle methods, while adapter tests retain wire ordering and refusal coverage.
