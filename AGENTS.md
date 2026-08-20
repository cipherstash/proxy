# Agent guidance

CipherStash Proxy is a Rust workspace providing transparent, searchable
encryption between applications and PostgreSQL.

## Domain context

Before exploring or changing package code, read `CONTEXT-MAP.md` and the
applicable package `CONTEXT.md`. Treat those files as the source of truth for
architecture, domain boundaries, request flow, and vocabulary.

## Development workflow

Use `mise tasks` to discover the current build, test, database, proxy, and
cross-compilation commands instead of duplicating that reference here. Run the
smallest relevant validation while iterating and the broader applicable checks
before completion.

### Coding conventions

- Define errors in `packages/cipherstash-proxy/src/error.rs`, grouped by
  problem domain rather than module. Use descriptive variant names without an
  `Error` suffix and give customer-facing errors helpful messages and
  documentation links.
- In tests, prefer `unwrap()` over `expect()` unless the expectation adds
  meaningful context. Prefer `assert_eq!` for equality checks.

### Release documentation

For user-facing or notable changes, update the `CHANGELOG.md` `[Unreleased]`
section using Keep a Changelog categories and user-facing language. For a
significant release, prepare `ANNOUNCEMENT.md` for GitHub Discussions and
remove that temporary file after publishing it.

## Pull request CI ownership

When a task authorizes pushing changes to a pull request, use passing required
GitHub checks as the completion criterion unless the user explicitly says not
to wait for CI.

After every push:

1. The main agent must spawn a background subagent whose role is **monitor
   only**. The monitor may inspect the PR, wait for checks with
   `gh pr checks --watch`, and collect failed-run logs with read-only `gh`
   commands. It must not edit files, run formatting that changes files, commit,
   push, or perform any other repository mutation.
2. The monitor must return the final check state and, for failures, the failed
   check names, run identifiers, and relevant failure output to the main agent.
3. The main agent is the sole writer. It must diagnose the failure, edit the
   current local worktree, run relevant validation, commit the fix, and push it
   to the PR branch.
4. After each fix push, the main agent must start a new monitor-only background
   subagent and repeat the loop.

The main agent may finish only when all required checks pass or when it reports
a genuine external blocker that it cannot resolve from the local worktree.

## Work tracking

- Issues live in Linear under Product Engineering (`CIP-`), not GitHub Issues.
  Read `docs/agents/issue-tracker.md` before creating, updating, or linking an
  issue.
- Read `docs/agents/triage-labels.md` before assigning or changing triage
  labels.
- Read `docs/agents/domain.md` when creating or maintaining domain context
  documentation.
