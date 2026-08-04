# pg-proto follow-ups

The proxy migration delegates framing, typed messages, startup/authentication
state, demultiplexing, and bounded pipeline scheduling to `pg-proto` 0.2.1.
Three remaining adapters would be better eliminated in `pg-proto` itself.

## Preserve buffered transport state across a split

`Buffered::into_inner()` cannot return retained inbound bytes, pending outbound
bytes, or demultiplexer state. The proxy must finish startup through
`ReadyForQuery` before splitting the bidirectional stream and then construct new
`Buffered` values for concurrent frontend/backend processing.

An `into_parts`/`from_parts` API, or a buffer-preserving split API, would let a
proxy change transport ownership without risking loss of bytes already read past
a message boundary. It should preserve both codec buffers and the demultiplexer.

## Accept application-provided SCRAM channel binding

The SCRAM-SHA-256-PLUS typestate obtains channel binding through pg-proto's TLS
transport trait. CipherStash uses its own `AsyncStream` TLS abstraction, which
already exposes the RFC 5929 binding bytes but cannot supply them through that
trait after type erasure.

Allowing callers to provide validated channel-binding bytes (or a small adapter
trait independent of the transport type) would remove the proxy's final manual
SCRAM-PLUS exchange. The SCRAM cryptographic engine itself should remain
application-owned.

## Transfer a demultiplexer between transport owners

The proxy routes backend messages through pg-proto's demultiplexer, but startup
and concurrent runtime currently use separate `Buffered` owners. A supported way
to extract and restore `Demux` state would retain startup parameter status,
cancellation-key, and readiness state without application bookkeeping.

This may naturally be solved by the buffer-preserving transport-parts API above.
