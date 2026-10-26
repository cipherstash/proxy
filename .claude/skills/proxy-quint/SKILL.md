---
name: proxy-quint
description: Proxy's rules for the Quint models in specs/quint. Use when changing a .qnt model, check.sh or a recorded finding, or when changing Rust code named in a model's source-file correspondence table.
---

# Quint models in Proxy

Read `specs/quint/README.md`, then the model's `requirements.md` and `verification.md`.
For Quint syntax and the CLI, use the `quint-lang` skill.
For modelling discipline and the spec review checklist, use the `quint-modeling` skill.
This skill holds only what is specific to Proxy, and wins where it differs from those two.

## Running the checks

- Iterate with `./check.sh` from `specs/quint`: typecheck, scenario tests, and simulation with every witness required to be reached.
- Before pushing a model change, run `./check.sh all`. TLC is part of Proxy's checks, unlike the kit's sampled-only workflow.
- Record the new state counts, timings and witness counts in `verification.md` in the same change.
- `check.sh` uses the TypeScript simulator backend, because the Rust backend downloads through the GitHub API, which cloud sessions block.

## Changing a model

1. Classify the change as a requirements change or an implementation change. A requirements change updates `requirements.md` and the model together, and its property needs review.
2. Treat every property as fixed. A failing check means the model is wrong or Proxy has a finding; fix the model or record the finding.
3. When modelling a code path, trace every call on it: the handler, every `Context::begin_*` and `finish_*` lifecycle hook, and `SettingsProtocol`. The path is traced when every function in the correspondence table for that element is accounted for. Finding F3 was a false alarm because the hand trace stopped at the handler.
4. For a new rule in the model, add a mutation to `check.sh` that removes it, and confirm TLC catches it.
5. For a new action, name it and call it directly from `step` (Quint Connect needs named actions), and add a witness that only that action can make true to the `simulate` list in `check.sh`.

## Findings

A counterexample is a hypothesis about Proxy until a failing test against Proxy and PostgreSQL reproduces it.
Record it in `requirements.md` under Findings and as an expected violation in `check.sh tlc`.
Fix Proxy in a separate pull request, test first.

## Changing Proxy's Rust code

When a change touches a function in a model's source-file correspondence table, update the model and rerun `./check.sh all` in the same change, or say in the pull request why the model is unaffected.

## Public repository

The model files, pull requests and commit messages are public: refer to work by GitHub PR number only, and keep Linear issue IDs out of them.
