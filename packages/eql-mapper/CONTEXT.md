# EQL Mapper

Parses SQL, infers a type for every node, and rewrites statements that touch encrypted
columns into their EQL v2 equivalents. It knows nothing about the PostgreSQL wire
protocol, ZeroKMS, or ciphertext — it reasons about types and rewrites syntax.

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
The identity of an encrypted column paired with the capabilities configured for it —
a `TableColumn` plus `EqlTraits`.

**EqlTrait**:
A class of operation an encrypted value supports: `Eq`, `Ord`, `TokenMatch`, `JsonLike`,
`Contain`. A **capability**, not a storage structure — several SEM terms can satisfy one
trait. See the shared vocabulary in `CONTEXT-MAP.md`.
_Avoid_: index, index type (those name the storage, not the capability).

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
A composable mutation of the typed AST that rewrites plaintext SQL into EQL v2
operations. Tuples of rules are themselves rules.
_Avoid_: using "rule" for a typing rule; those are operator/function signature
declarations.

**Param**:
A `$N` placeholder position in the statement. Its PostgreSQL type OID is Proxy's
concern, not this crate's.

## Known model gap

The trait-bounds machinery here is complete — `satisfy_bounds` and `UnsatisfiedBounds`
exist and work — but Proxy currently feeds every encrypted column `EqlTraits::all()`
(`packages/cipherstash-proxy/src/proxy/schema/manager.rs:146`) rather than the traits its
configured SEM terms actually provide. So bound violations cannot be caught at type-check
time in production today. Treat `EqlTraits` on an `EqlValue` as *intended* capability,
not observed capability, until that join exists.
