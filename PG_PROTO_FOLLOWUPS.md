# pg-proto follow-ups

The proxy migration delegates framing, typed messages, startup/authentication
state, typed startup middleware, async runtime middleware, demultiplexing, and
compile-time checked bounded pipeline dispatch to `pg-proto` 0.5.0. The
remaining transport adapters below would be better eliminated in `pg-proto`
itself.

The 0.5.0 version above is the historical baseline when these follow-ups were
written; the completed migration now uses pg-proto 0.11.1.

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

## Publish the example proxy driver as a library API

`pg-proto` demonstrates a clean `Buffered` + `Middleware` forwarding loop in
`examples/proxy_support`, but does not expose a configurable proxy driver from
the crate. CipherStash therefore still owns connection orchestration, concurrent
forwarding, and the small amount of glue that invokes middleware.

A library-level proxy builder should accept downstream/upstream transports,
startup and authentication policy, typed frontend/backend middleware, timeout
policy, and an output strategy. It should own framing, phase transitions,
bounded pipeline dispatch, demultiplexing, and shutdown. Applications would then
only supply policy and message transformations.
