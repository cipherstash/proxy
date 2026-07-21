---
status: accepted
---

# The v3 token type is inert identity data, not a checked type dimension

EQL v3 encrypted columns are two-dimensional — a token type (`integer`, `text`, …) crossed
with a capability (`_eq`, `_ord`, …) — where v2 had one opaque `eql_v2_encrypted`. We carry
the token type (and full domain) inside `EqlValue` as **inert identity data**: populated by
the schema loader from the Postgres domain name, read only at rewrite time to name the cast
target and select the term-extraction-function variant, and **not** a dimension the unifier
checks.

**Why not make it a checked dimension:** it would buy no safety. Two encrypted columns
never unify with each other — `TableColumn` identity already settles it (`unify_types.rs`),
so `users.email = orders.email` is already a type error regardless of token type. And a
plaintext literal is an inference sink: it starts as an unbound variable and *absorbs* the
column's type (`value.rs`), so there is nothing for `eql_v3_integer_ord = 'abc'` to
contradict at the literal. A checked token dimension would force the macro grammar, all six
unification arms, and the bound logic to learn a new axis for zero return. Capability
(`EqlTraits`) stays the only checked axis; the token type and the OPE-vs-ORE variant ride
in the identity and surface only during code generation.

## Consequences

- `EqlValue` becomes `(TableColumn, domain identity, EqlTraits)`; the identity threads
  through the associated-type machinery and unification arms for free because it is never
  inspected there.
- The source of truth for the identity is the Postgres domain name, parsed in
  `cipherstash-proxy`'s `SchemaManager` via the `eql-bindings` inventory; the encrypt
  config is demoted to a cross-check. `eql-mapper` stays wire-format-agnostic and takes no
  dependency on `eql-bindings`.
- If a future capability genuinely needs cross-column token-type checking (none does
  today), this decision is the thing to revisit.
