# Context Map

CipherStash Proxy is a Cargo workspace. Each context below owns a distinct domain
vocabulary. Read the `CONTEXT.md` for the context you're working in before exploring
its code.

Per-context `CONTEXT.md` files are created lazily by `/domain-modeling` as terms get
resolved — a missing one is expected, not a gap to fill upfront.

| Context | Path | Domain |
|---|---|---|
| Proxy | [`packages/cipherstash-proxy/`](./packages/cipherstash-proxy/CONTEXT.md) | PostgreSQL wire protocol, connection and message handling, client authentication, TLS, ZeroKMS key management, encrypt/decrypt of column values |
| EQL Mapper | [`packages/eql-mapper/`](./packages/eql-mapper/CONTEXT.md) | SQL parsing, type inference over statements, schema analysis, transformation rules that rewrite plaintext SQL into EQL v2 operations |
| Integration | `packages/cipherstash-proxy-integration/` | End-to-end test harness — container fixtures, encrypted-scenario coverage across the proxy and mapper together |
| Showcase | `packages/showcase/` | Healthcare example data model demonstrating EQL v2 encryption with realistic relationships |

`packages/eql-mapper-macros/` is proc-macro support for EQL Mapper, not a context of its
own — treat it as part of the EQL Mapper context.

## Relationships

- **Proxy → EQL Mapper**: Proxy loads the database schema and hands it over with each
  column marked native or encrypted; EQL Mapper returns a type-checked, rewritten
  statement. Proxy then reads the per-node EQL term shapes back out to decide how to
  encrypt each value.
- **Identity across the seam**: EQL Mapper's `TableColumn` and Proxy's `Identifier` are
  the same `table.column` pair under two names. That pair is the only key joining a typed
  AST node to its encryption config.
- **Capability across the seam — currently broken.** Proxy marks every encrypted column
  with *all* `EqlTrait`s (`packages/cipherstash-proxy/src/proxy/schema/manager.rs:146`)
  because it derives them from the PostgreSQL column type alone. The column encrypt
  config, which knows the SEM terms actually configured, is loaded by a separate manager
  that never meets the schema loader. EQL Mapper's bound checking is therefore
  unreachable in production: a query needing `Ord` on a column with no ordering SEM term
  type-checks cleanly and fails later. Read `EqlTraits` on a column as *intended*
  capability, not observed.

## Shared vocabulary

Terms defined once for the whole system live here rather than in any one context.

- **EQL v2** — Encrypt Query Language; the SQL-level encoding that makes encrypted
  values searchable.
- **`eql_v2_encrypted`** — the PostgreSQL column type holding an encrypted value.
- **ZeroKMS** — CipherStash's key management service, which the proxy calls to encrypt
  and decrypt.
- **Keyset** — the ZeroKMS key collection a workspace encrypts against.
- **SEM (searchable encrypted metadata)** — the encrypted material stored alongside a
  value that makes some operation on it possible without decryption. A **SEM term** is
  one such piece of metadata; ORE, unique, match and SteVec are SEM **types**.
- **EqlTrait** — a *capability* an encrypted column has: equality, ordering, token
  match, JSON traversal, containment. Several SEM types can satisfy the same trait, so
  the relationship is many-to-one: SEM terms are the storage, traits are what the storage
  buys you.

  Say **capability/trait** when you mean what a query may do, and **SEM term** when you
  mean what is written to the column. Do not call either an **index** — that word is
  reserved for a PostgreSQL index.

  > The EQL config JSON and `cipherstash_client`'s `ColumnConfig` still spell SEM terms
  > `indexes` / `IndexType`. That spelling is a wire and storage format shared with EQL
  > and customer-authored config, so it is not ours to rename unilaterally — prefer SEM
  > in prose and new code, and read `indexes` in those payloads as "SEM terms".

## Cross-context term collisions

Terms that mean different things depending on where you are. Resolve them against your
own context.

- **Type** — a PostgreSQL wire-protocol type in Proxy; an inferred SQL expression type
  in EQL Mapper.
- **Index** — means a PostgreSQL index in *both* contexts (EQL Mapper's only use of the
  word is GIN indexes). Where it appears meaning searchable encryption — the `indexes`
  key in EQL config, `IndexType`, "Unknown Index Term" — read it as **SEM term** and see
  the shared vocabulary above.
- **Column** — an encrypted column's runtime config in Proxy; a schema column or a
  projection column in EQL Mapper. Also `DataColumn` (a wire value) and
  `RowDescriptionField` (a result descriptor) in Proxy.
- **Statement** — a type-analysed statement in Proxy; the parsed AST or a
  `TypeCheckedStatement` in EQL Mapper. Four senses are live in Proxy alone.
- **Term** — the *shape* of an encrypted payload (`EqlTerm`) in EQL Mapper; a piece of
  searchable encrypted metadata in Proxy and EQL config. Qualify as *EQL term* or
  *SEM term*.
- **Projection** — a type in EQL Mapper's lattice; in Proxy, a positional list where a
  missing entry means "native, do not encrypt".
- **Session** — banned in Proxy (see its `CONTEXT.md`); PostgreSQL owns the word, and the
  code has used it for both a connection and a single statement.
