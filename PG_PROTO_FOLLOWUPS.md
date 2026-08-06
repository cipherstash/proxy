# pg-proto follow-ups

The proxy migration delegates framing, typed messages, startup/authentication
state, typed startup middleware, async runtime middleware, demultiplexing, and
bounded pipeline scheduling to `pg-proto` 0.3.0. The remaining adapters below
would be better eliminated in `pg-proto` itself.

## Integrate typed middleware with pipelined runtime sessions

CipherStash query mapping, encryption, and batched decryption are asynchronous.
Version 0.3.0 allows those operations to run inside async middleware. However,
`TypedMiddleware` derives its phase index from a single compile-time `Conn`
typestate. After startup, the proxy deliberately runs client-to-server and
server-to-client processing concurrently through `BoundedPipeline`, whose exact
projected phases are runtime-selected and may include multiple outstanding
operations.

The proxy therefore uses typed middleware throughout pre-startup, TLS,
authentication, and startup completion. Runtime rewriting uses async
direction-specific `MessageMiddleware`, followed by the bounded pipeline's
legality checks. Typed middleware hooks on `Pipeline` admissions and responses
would let those handlers receive and return phase-legal generated message types
without serializing or de-pipelining the two traffic directions. Those hooks
also need explicit outcomes for locally handled frontend operations and
suppressed/buffered backend messages, which cannot be represented by a
same-message-in/same-message-out middleware result.

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
