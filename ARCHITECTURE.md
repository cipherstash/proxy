# Architecture

This document describes the internal architecture of CipherStash Proxy. It's intended for anyone who wants to understand how the proxy pulls off transparent, searchable encryption without requiring application changes.

## Overview

CipherStash Proxy sits between an application and PostgreSQL. It intercepts SQL statements over the PostgreSQL wire protocol, determines which columns are encrypted, rewrites queries to use [EQL v2](https://github.com/cipherstash/encrypt-query-language) operations, encrypts literals and parameters, forwards the transformed query to PostgreSQL, and decrypts results before returning them to the application.

The two most interesting pieces of the system are:

1. **eql-mapper** — a SQL type inference and transformation engine that understands which parts of a query touch encrypted columns
2. **The protocol bridge** — a dual-stream PostgreSQL wire protocol interceptor that handles encryption and decryption transparently across both the simple and extended query protocols

## How a Query Flows Through the System

```
Application                    CipherStash Proxy                         PostgreSQL
    |                                |                                        |
    |--- SQL statement ------------->|                                        |
    |                          Parse SQL into AST                             |
    |                          Import schema (tables, columns, EQL types)     |
    |                          Run type inference (unification)               |
    |                          Identify encrypted literals & parameters       |
    |                          Encrypt values via ZeroKMS                     |
    |                          Apply transformation rules to AST             |
    |                          Emit rewritten SQL                             |
    |                                |--- transformed SQL ------------------>|
    |                                |<-- result rows ----------------------|
    |                          Identify encrypted columns in results          |
    |                          Batch-decrypt values via ZeroKMS              |
    |                          Re-encode to PostgreSQL wire format            |
    |<-- plaintext results ----------|                                        |
```

## SQL Type Inference Engine (eql-mapper)

The `eql-mapper` package is responsible for analyzing SQL statements and determining exactly which expressions, literals, and parameters need to be encrypted — and *how* they need to be encrypted. It does this through a constraint-based type inference system that operates entirely at parse time, without executing any SQL.

### The Type System

Every AST node in a parsed SQL statement is assigned a type. Types are either:

- **Native** — a standard PostgreSQL type. The proxy doesn't need to do anything special with these.
- **EQL** — an encrypted column type, carrying information about which operations it supports.
- **Projection** — an ordered list of column types (the result shape of a `SELECT`, subquery, or CTE).
- **Var** — an unresolved type variable, used during inference and resolved through unification.
- **Associated** — a type that depends on another type's trait implementation (e.g., "the tokenized form of this column").

EQL types carry **trait bounds** that describe what operations the encrypted column supports:

| Trait | Operations | Example |
|---|---|---|
| `Eq` | `=`, `<>` | `WHERE email = 'alice@example.com'` |
| `Ord` | `<`, `>`, `<=`, `>=`, `MIN`, `MAX` | `WHERE salary > 100000` |
| `TokenMatch` | `LIKE`, `ILIKE` | `WHERE name LIKE '%alice%'` |
| `JsonLike` | `->`, `->>`, `jsonb_path_query` | `WHERE data->>'role' = 'admin'` |
| `Contain` | `@>`, `<@` | `WHERE tags @> '["urgent"]'` |

Traits form a hierarchy — `Ord` implies `Eq`, and `JsonLike` implies both `Ord` and `Eq`.

### Unification

Type inference uses a **unification algorithm** (in the Robinson tradition, similar to what you'd find in a Hindley-Milner type system) adapted for SQL and encrypted types. When the type checker encounters an expression like `salary > 100000`, it:

1. Looks up `salary` in the current scope and finds its type (e.g., `EQL(employees.salary, Ord+Eq)`)
2. Assigns a fresh type variable to the literal `100000`
3. Looks up the `>` operator's type signature: `<T>(T > T) -> Native where T: Ord`
4. Unifies `T` with the salary's EQL type, checking that it satisfies the `Ord` bound
5. Unifies `T` with the literal's type variable, binding it to the same EQL type
6. Records that the literal `100000` must be encrypted as `EQL(employees.salary, Ord)`

This process propagates type information across the entire statement — through subqueries, CTEs, JOINs, `UNION` branches, function calls, and `RETURNING` clauses.

A particularly interesting aspect is how EQL types unify with each other. When two `Partial` EQL types for the same column meet, their bounds are merged (union). When a `Partial` meets a `Full`, the result promotes to `Full`. This means the system automatically infers the minimum encryption payload needed for each value.

### Polymorphic Function and Operator Signatures

SQL operators and functions are declared with generic type parameters and trait bounds using custom procedural macros:

```rust
binary_operators! {
    <T>(T =  T) -> Native where T: Eq;
    <T>(T <= T) -> Native where T: Ord;
    <T>(T -> <T as JsonLike>::Accessor) -> T where T: JsonLike;
    <T>(T ~~ <T as TokenMatch>::Tokenized) -> Native where T: TokenMatch;
}

functions! {
    pg_catalog.min<T>(T) -> T where T: Ord;
    pg_catalog.max<T>(T) -> T where T: Ord;
    pg_catalog.jsonb_path_query<T>(T, <T as JsonLike>::Path) -> T where T: JsonLike;
}
```

The `<T as JsonLike>::Accessor` syntax is an associated type — it resolves to `EqlTerm::JsonAccessor` when `T` is an EQL type with the `JsonLike` trait, or stays as `Native` when `T` is a native type. This lets the same operator signature work for both encrypted and unencrypted columns.

For unknown functions, the system falls back to assuming all arguments and the return type are native. This is a deliberately safe strategy: native types satisfy all trait bounds, so the type system never blocks a query it doesn't understand. Any actual type errors will be caught by PostgreSQL itself.

### Multi-Pass Single-Traversal Analysis

Three independent visitors operate in concert during a single AST traversal:

- **ScopeTracker** manages lexical scopes — tracking which tables, CTEs, and subquery aliases are visible at each point in the query. It handles column resolution, wildcard expansion (`SELECT *`), and qualified references (`t.column`).
- **Importer** brings schema information into scope. When the traversal enters a `FROM` clause, the importer resolves the table name against the schema and creates a typed projection for it, marking each column as either `Native` or `EQL` with the appropriate trait bounds.
- **TypeInferencer** performs the actual type inference using the unifier. It has specialized implementations for each AST node type — expressions, functions, `INSERT` column mappings, `SELECT` projections, set operations, and so on.

### In-Transaction DDL Tracking

When a SQL statement contains DDL (`CREATE TABLE`, `ALTER TABLE`, `DROP TABLE`, etc.), the eql-mapper captures these changes in a `SchemaWithEdits` overlay. This overlay acts as a mask over the loaded schema, so subsequent statements in the same transaction see the updated table structure. When the transaction commits, the proxy triggers a full schema reload.

## SQL Transformation Pipeline

After type inference determines which parts of a statement touch encrypted columns, the transformation pipeline rewrites the AST. Transformation rules are modular and composable — they implement a `TransformationRule` trait and are composed into a single rule via tuple implementation (supporting chains of 1 to 16 rules).

The current rules:

| Rule | What it does |
|---|---|
| `CastLiteralsAsEncrypted` | Replaces plaintext literals with `eql_v2.cast_as_encrypted(ciphertext)` |
| `CastParamsAsEncrypted` | Wraps parameter placeholders (`$1`, `$2`, ...) with encrypted casts |
| `RewriteContainmentOps` | Transforms `col @> val` to `eql_v2.jsonb_contains(col, val)` |
| `RewriteStandardSqlFnsOnEqlTypes` | Rewrites `min()`, `max()`, `jsonb_path_query()` etc. to `eql_v2.*` equivalents |
| `PreserveEffectiveAliases` | Maintains column aliases through transformations |
| `FailOnPlaceholderChange` | Postcondition check that prepared statement placeholders weren't corrupted |

Each rule has a `would_edit` method that tests whether it would modify the AST without actually modifying it. This enables a **dry-run optimization**: the system first checks if any rule would make changes, and only rebuilds the AST if necessary. For passthrough queries (those that don't touch any encrypted columns), this avoids the cost of AST reconstruction entirely.

## PostgreSQL Protocol Bridge

The proxy implements the PostgreSQL wire protocol, acting as both a server (to the application) and a client (to PostgreSQL). This is the `packages/cipherstash-proxy/` package.

### Dual-Stream Architecture

Each client connection gets a dedicated pair of handlers:

- **Frontend** (`frontend.rs`) — intercepts client-to-server messages, runs type inference and encryption on SQL statements, and forwards transformed messages to PostgreSQL.
- **Backend** (`backend.rs`) — intercepts server-to-client messages, identifies encrypted columns in result rows, decrypts values, and forwards plaintext results to the client.

These run concurrently on the same connection, connected by a shared `Context` that tracks session state (active statements, portals, column metadata, timing).

### Extended Query Protocol

The PostgreSQL extended query protocol separates SQL handling into distinct phases — Parse, Bind, Describe, Execute — with explicit Sync points. The proxy must track state across these phases:

- **Parse**: The proxy intercepts the SQL, runs type inference, encrypts any literals, transforms the AST, and forwards the rewritten SQL. It stores the type-checked statement metadata (column types, parameter types, projection) in the context.
- **Bind**: When parameters are bound to a prepared statement, the proxy looks up which parameters need encryption (from the Parse phase metadata), encrypts them, and forwards the modified Bind message.
- **Execute/Describe**: These are forwarded, with the backend using stored metadata to know which result columns need decryption.

Error recovery follows PostgreSQL semantics: when an error occurs, all messages are discarded until the next Sync message.

### Batch Decryption

Result rows containing encrypted data are buffered in a `MessageBuffer` (default capacity: 4096 rows) to enable efficient batch decryption. The buffer flushes when:

- It reaches capacity
- A non-DataRow message arrives (e.g., `CommandComplete`)
- The command completes

This batching reduces the number of decryption API round-trips. After decryption, values are re-encoded into the correct PostgreSQL wire format (text or binary) based on the format codes specified by the client.

### Authentication Bridging

The proxy handles authentication on both sides independently. It supports:

- MD5 password authentication
- SASL/SCRAM-SHA-256
- SCRAM-SHA-256-PLUS (with TLS channel binding)

The proxy authenticates the client using its own configured credentials, then separately authenticates with PostgreSQL using the database credentials. SSL/TLS negotiation is handled on both sides.

## Encryption and Key Management

Encryption operations go through CipherStash ZeroKMS. The proxy maintains a cache of `ScopedCipher` instances (keyed by keyset identifier) using a memory-weighted async cache with TTL eviction. Cache capacity is measured in bytes, not entry count.

### EQL Operation Routing

The type inference system determines not just *that* a value needs encryption, but *how*. Different EQL term variants map to different encryption operations:

| EQL Term | Encryption Operation | Use Case |
|---|---|---|
| `Full` | `EqlOperation::Store` | Inserting a new encrypted value with all search terms |
| `Partial(Eq)` | `EqlOperation::Store` | Equality query — only equality search terms needed |
| `Partial(Ord)` | `EqlOperation::Store` | Comparison query — only ORE search terms needed |
| `Tokenized` | `EqlOperation::Store` | LIKE query — tokenized search terms |
| `JsonPath` | `EqlOperation::Query` with `SteVecSelector` | JSON path query argument |
| `JsonAccessor` | `EqlOperation::Query` with field selector | JSON field access argument |

### Sparse Batch Encryption

When encrypting values for a statement, many columns may be `NULL` or non-encrypted. The proxy uses a sparse batch pattern: it collects only the non-null encrypted values (tracking their original positions), sends them to ZeroKMS in a single batch, then reconstructs the result vector with encrypted values placed back at their original positions. This minimizes API calls while handling nullable columns correctly.

## Schema Management

The proxy discovers the database schema at startup and reloads it periodically. Schema loading queries PostgreSQL's `information_schema` to discover tables and columns, then checks `eql_v2_configuration` to determine which columns are encrypted and what index types they support.

Schema state is stored behind an `ArcSwap`, which provides lock-free reads with atomic updates. This means query processing never blocks on a schema reload — readers always get a consistent snapshot.

The reload cycle:
1. **Startup** — load schema with exponential backoff retry (up to 10 attempts, max 2-second backoff) to handle cases where PostgreSQL isn't ready yet
2. **Periodic** — a background task reloads schema on a configurable interval
3. **On-demand** — DDL detection during a transaction triggers a reload when the transaction completes

## Package Structure

```
packages/
├── cipherstash-proxy/           # Main proxy binary
│   └── src/
│       ├── postgresql/          # Wire protocol implementation
│       │   ├── frontend.rs      # Client → Server message handling
│       │   ├── backend.rs       # Server → Client message handling
│       │   ├── handler.rs       # Connection startup and auth
│       │   ├── protocol.rs      # Low-level message reading
│       │   ├── parser.rs        # SQL parsing entry point
│       │   └── context/         # Session state (statements, portals, metadata)
│       ├── proxy/               # Encryption service, schema management, config
│       └── config/              # Configuration parsing
├── eql-mapper/                  # SQL type inference and transformation
│   └── src/
│       ├── inference/           # Type inference engine
│       │   ├── unifier/         # Unification algorithm, type definitions, trait bounds
│       │   ├── sql_types/       # Operator and function type signatures
│       │   └── infer_type_impls/# Per-AST-node type inference implementations
│       ├── transformation_rules/# AST rewriting rules
│       ├── model/               # Schema, tables, columns, DDL tracking
│       └── scope_tracker.rs     # Lexical scope management
├── eql-mapper-macros/           # Proc macros for operator/function declarations
└── showcase/                    # Example healthcare data model
```
