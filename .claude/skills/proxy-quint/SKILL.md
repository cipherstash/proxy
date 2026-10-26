---
name: proxy-quint
description: Proxy's rules for its Quint models in specs/quint. Use when adding, changing or reviewing a .qnt model, check.sh or a recorded finding, or when changing Proxy's statement routing, Parse/Bind lifecycle, settings protocol or schema reload.
---

# Quint models in Proxy

Read `specs/quint/README.md`, then the model's `requirements.md` and `verification.md`.
For Quint syntax and the CLI, use the `quint-lang` skill.
For modelling discipline and the spec review checklist, use the `quint-modeling` skill.
This skill holds only what is specific to Proxy, and wins where it differs from those two.

## Running the checks

- Iterate with `./check.sh` from `specs/quint`: typecheck, scenario tests, and simulation with every witness required to be reached.
- Before pushing a model change, run `./check.sh all` in the background (about 6 minutes) and wait for exit 0. TLC is part of Proxy's checks, unlike the kit's sampled-only workflow.
- Record the new state counts, timings and witness counts in `verification.md` in the same change.
- When you run `quint` yourself, use the pinned CLI (`npx --no-install quint` in `specs/quint`, or bare `quint` under mise) and pass `--backend typescript` to `run` and `test`: the default Rust backend downloads through the GitHub API, which cloud sessions block.

## Changing a model

1. Classify the change. A requirements change updates `requirements.md` and the model together, and the pull request description names each property it changes, for the reviewer to approve.
2. For an implementation change, the properties are fixed: a failing check means the model is wrong or Proxy has a finding. Fix the model, or record the finding.
3. When modelling a code path, follow every call from the handler that can refuse, rewrite or forward the message, through `Context` (every `begin_*`, `prepare_*` and `finish_*` hook) and `SettingsProtocol`, until it returns. The trace is done when every such function has a row in the model's source-file correspondence table. Add rows for new elements, and keep the file list in the `AGENTS.md` pointer in step with the table. Finding F3 was a false alarm because a hand trace stopped at the handler.
4. For a new rule in the model, add a mutation to `check.sh` that removes it, and confirm TLC catches it. `mutate` checks only the `asBuilt` instance and replaces the first match, so pick a pattern that occurs once, and extend `mutate` with an instance argument for a rule only another instance exercises.
5. For a new action, name it and call it directly from `step` (Quint Connect needs named actions), and add a witness that only that action can make true to the `simulate` list in `check.sh`.

## Findings

Record a counterexample as an unconfirmed finding, in four places:
- an entry under Findings in `requirements.md`
- an expected violation in `check.sh tlc`
- a row in `verification.md`
- a scenario test in the model's `*_test.qnt` that replays the trace

It stays unconfirmed until a failing test against Proxy and PostgreSQL reproduces it.
Fix Proxy in a separate pull request, test first.

## Changing Proxy's Rust code

When a change touches a function in a model's source-file correspondence table, update the model and rerun `./check.sh all` in the same change, or say in the pull request why the model is unaffected.

## Public repository

The model files are public, so refer to other work in them by GitHub pull request number.
