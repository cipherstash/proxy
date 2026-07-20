# Context Map

CipherStash Proxy is a Cargo workspace. Each context below owns a distinct domain
vocabulary. Read the `CONTEXT.md` for the context you're working in before exploring
its code.

Per-context `CONTEXT.md` files are created lazily by `/domain-modeling` as terms get
resolved — a missing one is expected, not a gap to fill upfront.

| Context | Path | Domain |
|---|---|---|
| Proxy | `packages/cipherstash-proxy/` | PostgreSQL wire protocol, session and message handling, client authentication, TLS, ZeroKMS key management, encrypt/decrypt of column values |
| EQL Mapper | `packages/eql-mapper/` | SQL parsing, type inference over statements, schema analysis, transformation rules that rewrite plaintext SQL into EQL v2 operations |
| Integration | `packages/cipherstash-proxy-integration/` | End-to-end test harness — container fixtures, encrypted-scenario coverage across the proxy and mapper together |
| Showcase | `packages/showcase/` | Healthcare example data model demonstrating EQL v2 encryption with realistic relationships |

`packages/eql-mapper-macros/` is proc-macro support for EQL Mapper, not a context of its
own — treat it as part of the EQL Mapper context.

## Shared vocabulary

Terms defined once for the whole system live here rather than in any one context.

- **EQL v2** — Encrypt Query Language; the SQL-level encoding that makes encrypted
  values searchable.
- **`eql_v2_encrypted`** — the PostgreSQL column type holding an encrypted value.
- **ZeroKMS** — CipherStash's key management service, which the proxy calls to encrypt
  and decrypt.
- **Keyset** — the ZeroKMS key collection a workspace encrypts against.

## Cross-context term collisions

Terms that mean different things depending on where you are. Resolve them against your
own context.

- **Type** — a PostgreSQL wire-protocol type in Proxy; an inferred SQL expression type
  in EQL Mapper.
- **Index** — a PostgreSQL index in Proxy; a searchable-encryption index (ORE, match,
  unique) in EQL Mapper and EQL config.
