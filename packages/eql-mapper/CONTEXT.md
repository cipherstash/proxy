# EQL Mapper

Parses SQL, infers a type for every node, and rewrites statements that touch encrypted
columns into their EQL v3 equivalents. It knows nothing about the PostgreSQL wire
protocol, ZeroKMS, or ciphertext — it reasons about types and rewrites syntax.

> **v3 migration.** The mapper now emits the EQL **v3** surface — v3 domain casts
> (`::public.eql_v3_*` for stored values, `::eql_v3.query_*` for operands) and the
> `eql_v3.*` functional-index form (term-extraction functions, `eql_v3.jsonb_*`,
> `eql_v3."->"`, `match_term`). No `eql_v2.*` names remain in its output.
> End-to-end validation against a live database with EQL v3 installed is still
> pending. See [`docs/adr/`](./docs/adr/) for the load-bearing decisions and
> `docs/plans/2026-07-20-eql-v3-type-checker-handoff.md` for the original impact maps.

## Language

### The type system

**Type**:
A node's inferred type in this crate's own lattice — either a resolved `Value`, a
unification `Var`, or an `Associated` type. Never a PostgreSQL wire type.
_Avoid_: SQL type, Postgres type (those belong to Proxy).

**Value**:
A *resolved* type — one of `Eql`, `Native`, `Array`, `Projection`, `SetOf`. It means
"value type", not a datum. The SQL AST's literal node is also called `Value`; when both
are in scope, alias the AST one.
_Avoid_: using bare `Value` for a SQL literal.

**Native**:
A column or expression that is not encrypted. Native types satisfy *every* `EqlTrait`
bound — that is a deliberate escape hatch for the type checker, not a claim about the
database.

**EqlValue**:
The identity of an encrypted column: a `TableColumn`, its **domain identity**, and its
`EqlTraits`. Two encrypted columns never share a type, so the `TableColumn` alone settles
unification — the domain identity and traits ride along for rewriting, not for checking.

**Domain identity**:
The inert `(token type, v3 domain)` an encrypted column carries — e.g. `text` /
`eql_v3_text_ord_ore`. Populated by the schema loader from the Postgres domain name,
never a checked dimension of unification. It does two jobs at rewrite time: names the
cast target and selects the **term-extraction function** variant (`ord_term` vs
`ord_term_ore`). It is the home of every v3 specific — token type, OPE-vs-ORE — that the
coarse `EqlTrait` deliberately does not carry.

**Token type**:
The plaintext scalar half of a v3 domain — `integer`, `text`, `timestamp`, … The
capability half (`_eq`, `_ord`, …) is the `EqlTraits`; together they name the domain.

**EqlTrait**:
A class of operation an encrypted value supports: `Eq`, `Ord`, `TokenMatch`, `JsonLike`.
A **capability**, not a storage structure — several SEM terms can satisfy one trait. It
is **coarse** by design: `Ord` says "ordering is allowed" without distinguishing OPE from
ORE, because that variant lives in the domain identity. See the shared vocabulary in
`CONTEXT-MAP.md`.
_Avoid_: index, index type (those name the storage, not the capability).

**`Contain`** is **retained** in v3, scoped to encrypted **JSON** columns
(`eql_v3_json_search`): `@>`/`<@` are real, supported operators there. On **scalar**
encrypted columns `@>`/`<@` raise — so `Contain` is a JSON-only capability, not a
general one. (An earlier note here claimed `Contain` was removed entirely in v3; that was
verified wrong against the installed `cipherstash-encrypt.sql`.)

**EqlTraits**:
A set of `EqlTrait`s. Read in two opposite directions depending on position: as
*required bounds* on a `Var`, and as *implemented capabilities* on an `EqlValue`.
_Avoid_: features, bounds, trait_impls as distinct concepts — they are all this one type.

**EqlTerm**:
The shape of encrypted payload a node needs — `Full`, `Partial`, `JsonAccessor`,
`JsonPath`, `Tokenized`. `Partial` is a subset of `Full`, not an unresolved type.
_Avoid_: "term" meaning an encrypted search term; that sense belongs to Proxy.

**Projection**:
The resultset shape of a statement or subquery — an ordered list of optionally-named
column types.

**alias**:
The *effective name* of a projection column, which is the user's alias when one was
written and the underlying schema column name otherwise. Identifier resolution matches
against it.
_Avoid_: reading `alias` as "user-supplied alias only".

### Naming and resolution

**Relation**:
Anything you can select from — a table, view, CTE, or derived subquery — reduced to a
name plus a projection type.

**Scope**:
The lexical frame holding the relations visible at a point in the statement.

**TableColumn**:
The `table.column` identity of an encrypted column. It is the join key to Proxy's
encrypt config.
_Avoid_: identifier (Proxy uses that for its own type).

**Schema**:
The database's tables and columns as this crate sees them, each column marked `Native`
or `Eql`. Loaded from the live database, not from a migration file.

**Overlay**:
A per-transaction mask over the loaded `Schema` recording DDL that has run but not
committed. A table is either shadowed by a new definition or hidden as dropped.

### Output

**TypeCheckedStatement**:
The result of type checking — the projection, params, literals and node types, plus the
entry point that applies transformations.

**TransformationRule**:
A composable mutation of the typed AST that rewrites plaintext SQL into EQL v3
operations. Tuples of rules are themselves rules.
_Avoid_: using "rule" for a typing rule; those are operator/function signature
declarations.

**Param**:
A `$N` placeholder position in the statement. Its PostgreSQL type OID is Proxy's
concern, not this crate's.

### EQL v3 rewriting

**Term-extraction function**:
An `eql_v3.*` function that pulls one SEM term out of an encrypted value into a natively
comparable scalar: `eq_term` → HMAC, `ord_term` → CLLW-OPE, `ord_term_ore` → block-ORE,
`match_term` → bloom filter. These are the target of rewriting, and the thing a functional
index is built on.

**Functional-index rewrite**:
The core v3 transformation: `col <op> x` becomes `term_fn(col) <op> term_fn(x)`, where
`term_fn` is chosen by the capability the operator exercises. It is the only form portable
to managed Postgres (Supabase forbids `CREATE OPERATOR`), and it lets a functional index
`CREATE INDEX ON t (eql_v3.ord_term(col))` engage via the term type's native opclass.
Selecting the term function *is* the capability check — a column whose domain provides no
such term is a capability error, not a fallback.
_Avoid_: describing this as "rewriting to operators"; v3 dispatches through functions.

**Query operand** (query twin):
The term-only payload used as the right-hand side of a rewritten predicate —
`{v, i, <terms>}`, the envelope minus the record ciphertext `c`. It inhabits the
`eql_v3.query_<name>` domains (the `query_<name>` twins), distinct from the `public`
column domains. A stored value is a full column domain; an operand is a query twin.

## Design decisions in flight

The trait-bounds machinery here is complete — `satisfy_bounds` and `UnsatisfiedBounds`
exist and work — but is **dead code today** because Proxy feeds every encrypted column
`EqlTraits::all()` (`packages/cipherstash-proxy/src/proxy/schema/manager.rs:146`).

The v3 type-checker extension closes this: the loader will source real per-column
`(token, capability)` from the Postgres domain name, making capability **observed**, not
intended, and turning bound-checking live. See the ADRs:

- [ADR-0001](./docs/adr/0001-functional-index-rewrite.md) — rewrite through
  term-extraction functions rather than native operators.
- [ADR-0002](./docs/adr/0002-token-type-as-inert-identity.md) — the token type is inert
  identity data, not a checked type dimension.
