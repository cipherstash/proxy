---
status: accepted
---

# The EQL v3 rewrite pipeline: term functions, cast targets, and operand context

ADR-0001 fixes *what* the mapper emits (the `eql_v3.*_term()` functional-index form);
this ADR fixes *how* the transformation pipeline produces it, because the v2→v3 change
is structural, not a find-and-replace of names.

## Two contexts, two cast targets

An encrypted value appears in a statement in one of two roles, and they cast differently:

- **Stored value** — an INSERT `VALUES` item or an UPDATE `SET` right-hand side. It casts
  to the column domain, `'<ciphertext>'::jsonb::public.eql_v3_<token>_<cap>`, and is **not**
  wrapped in a term function.
- **Query operand** — the right-hand side of a predicate (`col = $1`, `col > 'x'`). It casts
  to the query twin, `'<ciphertext>'::jsonb::eql_v3.query_<token>_<cap>`, and the whole
  predicate is rewritten through term functions (below).

The v2 pipeline did not need this distinction: every encrypted value cast to the single
opaque `eql_v2_encrypted`, and the opaque type carried its own operators. Under v3 the two
roles produce different SQL, so the pipeline must know a value's role.

**Decision:** the *type checker* records each encrypted literal/param's role while it walks
the AST (it already visits the INSERT/UPDATE targets and the predicate operands during
inference), and the transformation rules read that role. Re-deriving role from AST context
inside the transform is the fallback if threading it through inference proves awkward, but
the inference pass is where the context is already known.

## Operator rewriting

A comparison with an encrypted operand is rewritten by wrapping **both** operands in the
term function the operator's capability selects:

```
col <op> operand   →   eql_v3.<term>(col) <op> eql_v3.<term>(operand)
```

The term function is chosen by `(operator, the terms the column's domain stores)` — verified
against the `eql_v3.eq`/`lt`/… bodies in the installed `cipherstash-encrypt.sql`:

| Operator | Term function |
|---|---|
| `=` `<>` | `eq_term` **if the domain stores `hm`**, else `ord_term` (`op`), else `ord_term_ore` (`ob`) |
| `<` `<=` `>` `>=` | `ord_term` (domain stores `op`) / `ord_term_ore` (domain stores `ob`) |
| `@@` | `match_term` (domain stores `bf`) |

Two verified subtleties:

- **`=` is not always `eq_term`.** A term-extraction function exists exactly where its term
  does: `eq_term` only on domains storing `hm` (`_eq`, `_search*`, and — the text exception —
  `text_ord*`). On an ord-only scalar such as `integer_ord` there is no `hm`, so `eql_v3.eq`
  itself is `ord_term(a) = ord_term(b)`. The mapper mirrors this: `=`/`<>` fall back to the
  ordering term when the domain has no `hm`.
- **`ord_term` vs `ord_term_ore`** is not derivable from the coarse `Ord` trait — it comes
  from the domain identity (ADR-0002): a `*_ord` / `*_ord_ope` domain stores `op` ⇒
  `ord_term`; a `*_ord_ore` / `*_search_ore` domain stores `ob` ⇒ `ord_term_ore`.

Which terms a domain stores is recoverable from its typname (with the text `hm` exception);
the mapper derives them there rather than re-consulting `eql-bindings`.

### Operand cast target — the query twin

The right-hand operand casts to the **query twin** `eql_v3.query_<token>_<cap>` (schema
`eql_v3`), e.g. a `public.eql_v3_integer_ord` column's operand casts to
`eql_v3.query_integer_ord`. Verified: the twins exist for every scalar domain, carry the
**term-only** payload (`{v,i,<terms>}`, no stored ciphertext `c`) that a query value actually
is, and have their own `ord_term`/`eq_term` overloads. So:

```
salary > 'x'   →   eql_v3.ord_term(salary) > eql_v3.ord_term('<ct>'::jsonb::eql_v3.query_numeric_ord)
```

The column operand needs no cast (it is already the domain type); only the query operand is
cast, and to the twin — **not** the column domain, whose CHECK requires the ciphertext a
query value does not carry. (`eql_v3.eq(domain, jsonb)` casting to the column domain is a
separate convenience overload, not what the mapper emits.)

**Selecting the term function *is* the capability check.** A column whose domain provides no
term function for the operator (e.g. `ORDER BY` / `>` on an `_eq` column, any operator on a
storage-only `boolean`/`json` column) has no valid rewrite target — that absence *is* the
capability error the type checker raises. This keeps one mechanism, not a separate bounds
check bolted on.

## JSON is different (see ADR-0002 amendment)

Encrypted JSON columns (`eql_v3_json_search`) keep `->`/`->>` (JsonLike) **and** `@>`/`<@`
(Contain) — verified against the installed `cipherstash-encrypt.sql`. `Contain` is therefore
retained as a JSON-only capability and its rewrite targets the SteVec containment surface
(`eql_v3.to_ste_vec_query` + `@>`), **not** deleted. `@>`/`<@` on *scalar* encrypted columns
still have no term/rewrite and so raise.

## Rule inventory under v3

- `CastLiteralsAsEncrypted` / `CastParamsAsEncrypted` — retained; the cast target moves from
  `eql_v2_encrypted` to the role-appropriate v3 domain (column domain vs query twin). These
  gain access to each node's domain identity (via `node_types`) to name the target.
- **`RewriteEqlComparisonOps`** (new) — wraps scalar comparison operands in term functions
  and performs the capability check. Models its node handling on `RewriteContainmentOps`
  (`mem::replace` against a throwaway `Value::Null` to preserve operand `NodeKey` identity
  for the cast rules that run after).
- `RewriteContainmentOps` — **retargeted, not retired**: `@>`/`<@` on JSON columns rewrite to
  the v3 SteVec containment surface; on scalar columns they raise.
- `RewriteStandardSqlFnsOnEqlTypes` — retargeted from `eql_v2.{min,max,jsonb_*}` to the v3
  surface. Whether some of these become native overload resolution (and the rule shrinks) is
  gated on v3 shipping operator/function overloads bound to the domains; verify per function.
- `PreserveEffectiveAliases`, `FailOnPlaceholderChange` — unchanged.

## Consequences

- The pipeline gains a stored-vs-operand notion it did not have; this is the load-bearing new
  concept, and getting it wrong casts an operand to a column domain (or vice versa).
- Bound checking goes live here: the term-function selection raising on an absent capability
  is the user-visible capability error, so the ADR-0001 "let the database do its job" stance
  is refined — the mapper raises when there is no valid rewrite, rather than emitting SQL that
  would fail at the database.
