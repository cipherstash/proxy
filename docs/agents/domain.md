# Domain Docs

How the engineering skills should consume this repo's domain documentation when
exploring the codebase.

This is a **multi-context** repo — a Cargo workspace whose packages carry distinct
domain vocabularies.

## Before exploring, read these

- **`CONTEXT-MAP.md`** at the repo root — it points at one `CONTEXT.md` per context.
  Read each one relevant to the topic.
- **`docs/adr/`** — system-wide architectural decisions.
- **`packages/<name>/docs/adr/`** — context-scoped decisions for that package.

If any of these files don't exist, **proceed silently**. Don't flag their absence;
don't suggest creating them upfront. The `/domain-modeling` skill (reached via
`/grill-with-docs` and `/improve-codebase-architecture`) creates them lazily when terms
or decisions actually get resolved.

Note that `ARCHITECTURE.md` at the repo root already describes system structure. It is
not a glossary — read it for orientation, but domain *terms* belong in `CONTEXT.md`.

## File structure

This repo is a Cargo workspace, so contexts live under `packages/`, not `src/`:

```
/
├── CONTEXT-MAP.md
├── docs/adr/                                  ← system-wide decisions
└── packages/
    ├── cipherstash-proxy/
    │   ├── CONTEXT.md
    │   └── docs/adr/                          ← context-specific decisions
    └── eql-mapper/
        ├── CONTEXT.md
        └── docs/adr/
```

## Use the glossary's vocabulary

When your output names a domain concept (in an issue title, a refactor proposal, a
hypothesis, a test name), use the term as defined in the relevant `CONTEXT.md`. Don't
drift to synonyms the glossary explicitly avoids.

If the concept you need isn't in the glossary yet, that's a signal — either you're
inventing language the project doesn't use (reconsider) or there's a real gap (note it
for `/domain-modeling`).

Watch for terms that mean different things in different contexts. "Type", for example,
means a PostgreSQL wire type in `cipherstash-proxy` and an inferred SQL type in
`eql-mapper`. That divergence is exactly why this repo is multi-context — resolve the
term against the context you're working in, not the other one.

## Flag ADR conflicts

If your output contradicts an existing ADR, surface it explicitly rather than silently
overriding:

> _Contradicts ADR-0007 (event-sourced orders) — but worth reopening because…_
