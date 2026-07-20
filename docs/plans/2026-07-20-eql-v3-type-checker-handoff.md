# EQL v3 type checker — session handoff

**Date:** 2026-07-20
**Branch:** `chore/agent-skills-setup` (PR #415)
**Purpose:** carry context into a fresh session for `/grill-with-docs` on extending the
EQL Mapper type checker for EQL v3.

This is a handoff, not a spec. It records what is already true, what was already decided,
and what is still open. The design itself has not started.

---

## 1. How to use this

Open a fresh session, reference this file, and run `/grill-with-docs`. The design questions
in §8 are the agenda. Everything above them is evidence gathered so the grill does not have
to re-derive it.

Glossaries exist and should be updated as terms get resolved:
`CONTEXT-MAP.md`, `packages/eql-mapper/CONTEXT.md`, `packages/cipherstash-proxy/CONTEXT.md`.

---

## 2. Where the branch is

Five commits, unpushed beyond #415's first. In order:

| Commit | What |
|---|---|
| `3d33c8ed` | Agent skills config (issue tracker, domain docs) |
| `74b63721` | Domain glossaries for Proxy and EQL Mapper |
| `64c658f2` | Deps: cipherstash-client `0.34.1-alpha.4` → `0.42.0`, EQL `2.3.0-pre.3` → `3.0.1` |
| `b3f293d7` | Encrypt path moved to `encrypt_eql_v3`; v2 retired |
| `a10b60cd` | v3 decrypt for scalar + SteVec |
| `387ca790` | Adopted the 0.42.0 representation; workspace compiles |

**State:** the workspace compiles, `cargo fmt` and `cargo clippy --all-targets` are clean.
**Nothing has been run.** Compiling is not working — `eql-mapper` still emits `eql_v2_encrypted`
casts and `eql_v2.*` calls into every rewritten statement, so nothing works end to end.
229 `eql_v2` references remain repo-wide.

The vendored `stack-auth` CIP-3159 patch was dropped (0.42.0 requires `stack-auth ^0.42.0`,
which carries the CancelGuard fix upstream). `vendor/stack-auth/` is still on disk, unreferenced.

---

## 3. The EQL v3 model

EQL 3.0 removes the `eql_v2` schema entirely. The single opaque `eql_v2_encrypted`
**composite** type is replaced by **53 typed domains over jsonb**, two-dimensional:

**Token type** × **capability suffix**

- Tokens: `integer`, `smallint`, `bigint`, `date`, `timestamp`, `numeric`, `text`,
  `boolean`, `real`, `double`, `json`
- Suffixes: *(none)*, `_eq`, `_ord`, `_ord_ope`, `_ord_ore`, `_match`, `_search`, `_search_ore`

Column domains live in `public` (`public.eql_v3_integer_ord`); query-operand twins live in
`eql_v3` (`eql_v3.query_integer_ord`). 39 query twins, enumerated separately.

Capability is a property of the **type**, and the domain's CHECK enforces that the payload
actually carries the required terms — so a missing term fails on insert, not later at query time.

### SEM terms (the storage beneath a capability)

| Suffix | Operators | Term |
|---|---|---|
| *(none)* | — (storage only) | `c` |
| `_eq` | `=` `<>` | `hm` (HMAC-256) |
| `_ord` | `=` `<>` `<` `<=` `>` `>=`, MIN/MAX | `op` (CLLW-OPE) |
| `_ord_ope` | as `_ord` | `op` |
| `_ord_ore` | as `_ord` | `ob` (block-ORE) |
| `_match` | `@@` | `bf` (bloom filter) |
| `_search` | all | `hm` + `op` + `bf` |
| `_search_ore` | all | `hm` + `ob` + `bf` |

Text is the exception: `text_ord*` carries `hm` **as well as** the ordering term, because
lexicographic ORE/OPE over text is not equality-lossless.

> **Upstream doc bug.** `eql-bindings` `src/v3/mod.rs:26` claims "`ob` for `_ord`/`_ord_ore`,
> `op` for `_ord_ope`". The generated code disagrees — `IntegerOrd` requires `op`,
> `IntegerOrdOre` requires `ob`. Trust `term_json_keys_static()`, not that module doc.

### Vocabulary

`EqlTrait` is a **capability**; a **SEM term** (searchable encrypted metadata) is the storage
that satisfies it. Many-to-one. Neither is an "index" — that word is reserved for a PostgreSQL
index. The EQL config JSON and `cipherstash_client`'s `ColumnConfig` still spell SEM terms
`indexes`/`IndexType`; that is a shared wire format and not ours to rename.

---

## 4. Upstream facts worth knowing

**`eql-bindings` 3.0.1** (crates.io) is generated from `eql-domains::CATALOG` by `eql-codegen` —
the same catalog that generates the SQL. Light deps (serde, serde_json, schemars, ts-rs).

The useful part for us:

```rust
IntegerOrd::sql_domain_static()      // "public.eql_v3_integer_ord"
IntegerOrd::term_json_keys_static()  // Some(&["op"])
```

`v3::all()` enumerates the 53 stored domains, `all_query()` the 39 query twins. Inverting
`all()` gives a **typname → (token type, required SEM terms)** map straight from the catalog,
which cannot drift from the SQL. `sql.rs` also embeds `INSTALL_SQL`/`UNINSTALL_SQL` as consts,
which could replace the `curl` in `mise.toml`'s `eql:download`.

**Containment is gone.** `@>`/`<@` survive only as internal blockers that raise (CIP-3517).
Fuzzy match is `@@` via `eql_v3.matches`.

**`boolean` is storage-only by design** — no `_eq`, no `_ord`, every operator blocked, because
a two-value column leaks its distribution under any index.

### The 0.41.1 / 0.42.0 SteVec split — UNRESOLVED

cipherstash-client changed the v3 SteVec wire format and nothing else followed:

| | 0.41.1 (protect-ffi, eql-bindings 3.0.1) | 0.42.0 (what Proxy now pins) |
|---|---|---|
| Document | `{v,k,i,sv}` | `{v,k,i,**h**,sv}` |
| Entry `c` | self-describing mp_base85 record | raw base85 AEAD bytes |
| Key material | per entry | once, in `h` |
| Nonce / AAD | data key IV | `selector[..12]` / all 16 selector bytes |

EQL 3.0.1's SQL CHECK accepts **both** (it never forbids extra keys), so the database will not
catch a mismatch. ProtectJS and Proxy writing encrypted jsonb to the same table would produce
mutually undecryptable documents. Either Proxy pins back to 0.41.1 or protect-ffi and
eql-bindings move forward. Not Proxy's call alone. **This is orthogonal to the type checker
work but blocks shipping.**

### CLLW-ORE data is stranded

`from_v2` returns `UnconvertibleOreTerm` for `oc` SteVec entries — re-encryption is the only
path. `cipherstash-config` now models this: `IndexType::SteVec { mode }` where
`SteVecMode::Compat` (CLLW-OPE, `op`) is the default and v3-compatible, and `SteVecMode::Standard`
(CLLW-ORE, `oc`) is documented upstream as "the legacy v2 protocol, still used by Proxy".
This is CIP-3233 made explicit in the config model.

---

## 5. Impact map — the type system

Three parallel agents surveyed `eql-mapper`. Findings, with the conflicts resolved.

### Where encrypted types come from

**Only two production sites** construct an `EqlValue` from schema data:

- `inference/unifier/types.rs:508-517` — `Projection::new_from_schema_table`
- `inference/infer_type_impls/insert_statement.rs:40-49` — INSERT target columns

Both produce `EqlTerm::Full`. Everything else clones. Upstream of both:
`packages/cipherstash-proxy/src/proxy/schema/manager.rs:142-148`, which matches the single
string `"eql_v2_encrypted"` and hardcodes `EqlTraits::all()`. **That one match arm is where the
53-domain mapping has to go.**

### The shapes that must widen

- `EqlValue(TableColumn, EqlTraits)` — `unifier/types.rs:274`
- `ColumnKind::{Native, Eql(EqlTraits)}` — `model/schema.rs:41-45`
- `Column::eql(name, features)` — `model/schema.rs:48`
- `SchemaTableColumn` — `model/schema.rs:63`

There is **no field anywhere in eql-mapper holding a plaintext scalar type for an encrypted
column.** The `_ord` half of `eql_v3_integer_ord` is derivable from `EqlTraits`; the `integer`
half is derivable from nothing in the package.

### Two authorities on the scalar type, no reconciliation

The scalar type already exists in the system as `ColumnType`, sourced from the **encrypt config**
and consumed at `cipherstash-proxy/src/postgresql/data/from_sql.rs:123-292` and
`context/column.rs:68-86`. Under v3 the **Postgres type name** also encodes it. A config/schema
disagreement is currently undetectable.

### Encrypted columns never unify with each other

`unifier/types.rs:121` — *"An encrypted column never shares a type with another encrypted column."*
`unify_types.rs:84-121`: two EQL terms unify iff their `EqlValue`s are `==`, so `TableColumn`
identity dominates. `users.email = orders.email` is already a type error.

**Therefore a token type is redundant for `Full`/`Full` unification.** Same column → same token
type, always.

### Literals are inference sinks — CORRECTED FINDING

Two agents disagreed here; resolved by reading the code.

`infer_type_impls/value.rs:19` — a non-placeholder literal unifies with `self.fresh_tvar()`,
an **unbound variable**, not `Native`. And `(Eql, Native)` is a hard error
(`unify_types.rs:68-70`).

So `WHERE encrypted_col = 'literal'` works because the literal has no type of its own and
**absorbs the column's**. Consequence: adding a token type to `EqlValue` buys **zero** literal
checking — there is nothing for `eql_v3_integer_ord = 'abc'` to contradict. Getting that check
would require literals to carry a syntactic type they deliberately do not have.

This is not the same as "Native satisfies all bounds". That escape hatch
(`unifier/eql_traits.rs:279`) is about **bound satisfaction**, not cross-kind unification.

### Bound checking is dead code today

`Unifier::satisfy_bounds` (`unifier/mod.rs:235-248`) and `Type::must_implement`
(`unifier/types.rs:451-460`) can never fail in production, because:

- every column gets `EqlTraits::all()` (`manager.rs:146`)
- `Native` ⇒ `ALL_TRAITS` (`eql_traits.rs:279`)
- `Type::Var` short-circuits to `Ok(())` (`mod.rs:236`)

**Two latent bugs will become user-visible the moment these go live:**

1. `EqlTraits::difference` (`eql_traits.rs:223-231`) is implemented as **XOR**, not set
   difference. `UnsatisfiedBounds` will report the symmetric difference — listing traits the
   type *has* but the bound never required.
2. `must_implement` (`types.rs:456`) passes its operands **reversed** relative to
   `satisfy_bounds` (`mod.rs:245`). Harmless only because XOR is commutative.

Fix both before making bounds reachable.

### Associated types

`AssociatedTypeSelector { eql_trait, type_name }` (`types.rs:236`) is keyed on
`(EqlTrait, &'static str)` only. Resolution (`eql_traits.rs:56-115`) maps
`Eq::Only`/`Ord::Only`/`Contain::Only` → `Partial`, `TokenMatch::Tokenized` → `Tokenized`,
`JsonLike::{Path,Accessor}` → `JsonPath`/`JsonAccessor`.

**If the token type lives inside `EqlValue`, this machinery needs no change** — every arm does
`eql_col.clone()`, so it propagates free. Do **not** add a field to `AssociatedTypeSelector`:
it would break the `lhs.selector == rhs.selector` equality at `unify_types.rs:160` for a
dimension already determined by `impl_ty`.

---

## 6. Impact map — the SQL surface

### The entire v2 SQL surface is three call sites

Six rules, composed at `type_checked_statement.rs:153-160`. Only three emit SQL.

**`helpers.rs:6-27`** — `cast_as_encrypted`. The single source of every
`::JSONB::eql_v2_encrypted` cast. Takes **only** an `ast::Value` — no `EqlTerm`, no
`TableColumn`, no traits. The cast target is a hardcoded `Ident::new("eql_v2_encrypted")`
at line 17. **This is the one place the v3 domain name would be chosen.**

**`cast_params_as_encrypted.rs:51`** — emits `$1::JSONB::eql_v2_encrypted`. Its gate at line 67
matches `Some(Type::Value(Value::Eql(_)))` — **the `EqlTerm` is available and discarded by the
`_`, one line before use.** That is the hook for selecting a `query_<name>` twin.

**`cast_literals_as_encrypted.rs:33`** — emits `'<ciphertext>'::JSONB::eql_v2_encrypted`. Gated
on map membership, not type. Holds **no `node_types` field at all** (line 13), so it cannot see
type info even in principle. Needs plumbing, not just a pattern change.

**`rewrite_standard_sql_fns_on_eql_types.rs:59-61`** — blindly prepends `eql_v2` to any
`pg_catalog.*` name. Emits `eql_v2.{count,min,max,jsonb_path_query,jsonb_path_query_first,
jsonb_path_exists,jsonb_array_length,jsonb_array_elements,jsonb_array_elements_text}`.
Note it emits `eql_v2.count`, which **is not in the declaration table** — a name the type
registry cannot round-trip.

**`rewrite_containment_ops.rs:54-57,86-87`** — `@>` → `eql_v2.jsonb_contains`,
`<@` → `eql_v2.jsonb_contained_by`. Motivated by GIN index usage, not just type dispatch.

`preserve_effective_aliases.rs` and `fail_on_placeholder_change.rs` emit nothing.

### Rules that exist only because v2 had one opaque type

`RewriteStandardSqlFnsOnEqlTypes` — PG cannot dispatch `min`/`max`/`jsonb_*` on one opaque
jsonb-backed type, so they are shadowed by hand-written `eql_v2.` overloads. Under v3, PG
resolves overloads by argument type natively — **but only if v3 ships operator/function
overloads bound to those domains.** Redundant *conditional on* that, and that is overload
resolution, not capability-in-the-type. Different mechanisms; verify before assuming.

`RewriteContainmentOps` — same root cause: `@>` cannot be defined on a bare opaque type
without conflicting with jsonb's own operator.

**Not in this category:** the two cast rules (still needed, only the target changes),
`PreserveEffectiveAliases` (projection naming, orthogonal), `FailOnPlaceholderChange` (assertion).

### Discipline to preserve

`rewrite_containment_ops.rs:92-99` uses `mem::replace` against a throwaway `Value::Null` rather
than cloning, specifically so operand `NodeKey` identity survives for the cast rules that run
after it. **Clone there and literal encryption inside containment expressions silently breaks.**

---

## 7. Impact map — declarations and macros

### Operator → EqlTrait, in full (`inference/sql_types/sql_decls.rs:16-31`)

| Operator | Signature | Trait |
|---|---|---|
| `=` `<>` | `<T>(T op T) -> Native` | `Eq` |
| `<` `<=` `>` `>=` | `<T>(T op T) -> Native` | `Ord` |
| `->` `->>` | `<T>(T op <T as JsonLike>::Accessor) -> T` | `JsonLike` |
| `@>` `<@` | `<T>(T op T) -> Native` | `Contain` |
| `~~` `!~~` `~~*` `!~~*` | `<T>(T op <T as TokenMatch>::Tokenized) -> Native` | `TokenMatch` |

No `@@` is declared. Anything unlisted falls to `Fallback`, which forces lhs, rhs and result
to `Type::native()` (`sql_binary_operator_types.rs:40-44`).

Functions at `sql_decls.rs:58-79`: `min`/`max` require `Ord`; the `jsonb_*` family requires
`JsonLike`; `jsonb_array`/`jsonb_contains`/`jsonb_contained_by` require `Contain`; **`count`
has no bound at all** and accepts any `T` including EQL.

### The macro grammar is capability-only by construction

`eql-mapper-macros/src/parse_type_decl.rs:11-25` — the entire custom-keyword vocabulary is
`Accessor, Contain, EQL, Eq, Full, JsonLike, Native, Only, Ord, Partial, Path, SetOf, TokenMatch`.
Every one is a trait name, an associated-type name, or a type constructor. **Not one names a
data type.**

`EQL(customer.age: Ord)` is expressible. "Encrypted integer with Ord" is not — not in the
keyword table, not in the grammar, not in the generated `EqlValue`. A v3 port needs a new
lexical class, new productions in `EqlTerm` (`:289-331`) and `VarDecl` (`:70-91`), and a
widened payload. Work is concentrated in that one file.

`@@` will not parse today: `token::At` at `:480` unconditionally expects a following `Gt`.

### Things that become ambiguous with a token type

1. **`<T>(T = T)` — the core ambiguity.** One tvar currently conflates *same column*, *same
   token type*, and *any encrypted*. Indistinguishable today because encrypted identity **is**
   the column.
2. **`EqlTerm::Partial`'s union-on-unify** (`unify_types.rs:91`) accumulates capabilities
   monotonically. If capability is fixed by the domain, you cannot union your way into a
   capability the column does not have.
3. **The payload-less variants** — `JsonAccessor`, `JsonPath`, `Tokenized` all return
   `EqlTraits::none()` (`eql_traits.rs:320-322`). "A tokenized *what*" needs answering.
4. **`Native` ⇒ `ALL_TRAITS`** and **`Type::Var` ⇒ `Ok(())`** are two independent wildcards in
   the bound checker.

### Pre-existing gaps this work will expose

- **`Expr::Like`/`ILike` bypass the trait system entirely.** `infer_type_impls/expr.rs:115-131`
  only unifies the result with `Native` — never requires `TokenMatch`, never produces a
  `Tokenized` term. The `~~` declarations fire only on the operator form. LIKE on an encrypted
  column is currently unchecked.
- **`schema_delta.rs:447,479,529` uses `EqlTraits::default()`** (all false) while the loader uses
  `all()` (all true). The two paths disagree. Invisible while bounds are dead.
- **Empty projections** return `EqlTraits::none()` (`eql_traits.rs:310`) while the
  `satisfy_bounds` doc (`mod.rs:220`) says they satisfy all bounds. Doc and code disagree.
- `EqlTrait` parse error string (`parse_type_decl.rs:172`) is stale — omits `Contain`.

### Removing `Contain` touches

Keyword table (`parse_type_decl.rs:13`), enum (`eql_traits.rs:23`), assoc types (`:39`),
resolution (`:73`, `:101-105`), `ALL_TRAITS` (`:154`), struct field (`:145`), `add_mut` (`:200`),
`to_eql_trait_impls!` (`schema.rs:225-227`), two operators (`sql_decls.rs:25-26`), three
functions (`sql_decls.rs:76-78`).

---

## 8. Open design questions — the grill agenda

**The framing question.** The research says the token type's job is **naming a v3 domain at
cast time**, not type checking: it is redundant for unification (§5), and it buys no literal
checking (§5). So — does it belong in the type lattice at all, or should the transformation
rules look the domain up from the schema by `TableColumn` at emit time and leave the lattice
alone? The second is materially cheaper and nothing found so far rules it out. **Settle this
before anything else; most other answers follow from it.**

Then:

1. If it does go in the lattice: inside `EqlValue` (associated types come free, all six
   unification arms come free) or on `EqlTerm` (six explicit merges)?
2. `_ord` vs `_ord_ope` vs `_ord_ore` — identical operators, different SEM terms. Does the type
   checker distinguish them, or is that purely a cast-target concern?
3. `integer_eq` and `bigint_eq` are byte-identical on the wire but different domains. Type error
   to compare? (Note: already a type error via `TableColumn` identity — is that enough?)
4. What replaces `Contain`? `@>`/`<@` are **directional**; `@@`/`eql_v3.matches` is **symmetric**.
   `<@` has nowhere to go. Delete the trait, or repoint at `@@` and lose direction?
5. Do bounds go live? Real errors for `ORDER BY` on an `_eq` column — but the domain CHECK
   catches it anyway, and `sql_decls.rs:49` states the existing philosophy is *"let the database
   do its job."* Does v3 change that stance?
6. Params must cast to `eql_v3.query_<name>` twins, not the stored domain. Does the mapper need
   the query inventory too, or can the twin name be derived by prefixing?
7. Which authority owns an encrypted column's scalar type — `pg_type` or the encrypt config?
   Should the other be cross-checked?
8. Does `eql-bindings` become a dependency of `eql-mapper` (inverting `all()` into a
   typname → capability map), or does the mapper keep its own table?

### Prerequisites, whatever the design

- Fix `EqlTraits::difference` XOR bug and the reversed `must_implement` operands **before**
  bounds become reachable.
- Reconcile `schema_delta.rs`'s `default()` with the loader's `all()`.
- Verify whether EQL 3 ships operator/function overloads bound to the domains — that alone
  decides whether `RewriteStandardSqlFnsOnEqlTypes` retires.

---

## 9. Related memory

`~/.claude/projects/-Users-jamessadler-cipherstash-proxy/memory/`:
`eql-3-upgrade-in-flight.md`, `cip-3233-client-038-ore-cllw-break.md`,
`proxy-222-stackauth-15min-auth-bug.md`.
