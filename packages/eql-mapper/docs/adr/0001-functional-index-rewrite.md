---
status: accepted
---

# Rewrite encrypted operators through term-extraction functions, not native operators

EQL v3 ships native operators (`CREATE OPERATOR public.=`, btree opclasses) bound to its
53 domain types, which makes it tempting to leave comparisons untouched and let Postgres
dispatch them. We will **not** do that. The mapper rewrites every operator on an encrypted
column into the functional form `eql_v3.<cap>_term(lhs) <native-op> eql_v3.<cap>_term(rhs)`
— `eq_term` (→ HMAC), `ord_term` / `ord_term_ore` (→ CLLW-OPE / block-ORE), `match_term`
(→ bloom filter) — with the term function chosen by the capability the operator exercises.

**Why:** our primary deployment target is Supabase and other managed Postgres, where
`CREATE OPERATOR` DDL is unavailable to non-superusers, so the native-operator path simply
does not exist there. The functional form is the only surface portable across every
install, it engages functional indexes (`CREATE INDEX ON t (eql_v3.ord_term(col))`) via
the term type's native opclass, and we have tested that Postgres's query planner handles
it well. This also collapses two concerns into one: **selecting the term function is the
capability check** — a column whose domain provides no matching term (e.g. `ORDER BY` on an
`eql_v3_text_eq` column, or any operator on storage-only `eql_v3_boolean` / `eql_v3_json`)
has no valid rewrite target, which is exactly the type error we raise.

## Consequences

- The v2 transformation layer does **not** evaporate under v3 — it is retargeted from the
  `eql_v2.*` opaque-type functions to the `eql_v3.*_term()` functional-index surface. This
  is close to the v2 structure, not a rewrite from scratch.
- The `_ord` vs `_ord_ore` distinction (OPE `op` vs block-ORE `ob`) becomes load-bearing:
  the mapper must emit `ord_term` vs `ord_term_ore` per the column's domain. That variant
  is read from the domain identity (see ADR-0002), not from the coarse `Ord` trait.
- Native `@>`/`<@`/operators on **plaintext** columns are untouched; only encrypted
  operands are rewritten.
