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
