# pg-proto follow-ups

The proxy migration delegates framing, typed messages, startup/authentication
state, typed startup middleware, demultiplexing, and bounded pipeline scheduling
to `pg-proto` 0.2.3. The remaining adapters below would be better eliminated in
`pg-proto` itself.

## Support asynchronous typed middleware for pipelined runtime sessions

CipherStash query mapping, encryption, and batched decryption are asynchronous.
`TypedMiddleware::intercept_typed` is synchronous, and its phase index is derived
from a compile-time `Conn` typestate. After startup, the proxy deliberately runs
client-to-server and server-to-client processing concurrently through
`BoundedPipeline`, whose exact projected phases are runtime-selected.

The proxy therefore uses typed middleware throughout pre-startup, TLS,
authentication, and startup completion, but retains its asynchronous runtime
rewrite handlers behind the bounded pipeline's legality checks. An asynchronous
typed middleware interface integrated with `Pipeline` admissions and responses
would let those handlers receive and return phase-legal generated message types
without serializing the two traffic directions.

## Preserve buffered transport state across a split

`Buffered::into_inner()` cannot return retained inbound bytes, pending outbound
bytes, or demultiplexer state. The proxy must finish startup through
`ReadyForQuery` before splitting the bidirectional stream and then construct new
`Buffered` values for concurrent frontend/backend processing.

An `into_parts`/`from_parts` API, or a buffer-preserving split API, would let a
proxy change transport ownership without risking loss of bytes already read past
a message boundary. It should preserve both codec buffers and the demultiplexer.

## Transfer a demultiplexer between transport owners

The proxy routes backend messages through pg-proto's demultiplexer, but startup
and concurrent runtime currently use separate `Buffered` owners. A supported way
to extract and restore `Demux` state would retain startup parameter status,
cancellation-key, and readiness state without application bookkeeping.

This may naturally be solved by the buffer-preserving transport-parts API above.
