# Issue Tracker

Issues for this repo live in **Linear**, not GitHub Issues.

- **Workspace:** CipherStash
- **Team:** Product Engineering
- **Issue key prefix:** `CIP-` (e.g. `CIP-3233`)

GitHub (`cipherstash/proxy`) holds code, branches, and pull requests. It is not the
issue tracker. Do not run `gh issue create` for this repo.

## How to read and write issues

Two access paths exist. **Prefer the CLI for writes.**

### `linear-cli` (preferred)

The `linear` command (`@schpet/linear-cli`, or `npx @schpet/linear-cli`) — see the
`linear-cli` skill for full usage.

```bash
linear issue list
linear issue view CIP-3233
linear issue create --title "..." --description-file /path/to/desc.md
linear issue update CIP-3233 --status "In Progress"
```

Always pass markdown via `--description-file` / `--body-file` rather than inline
arguments — inline content produces literal `\n` sequences in the Linear UI.

### Linear MCP tools (fallback, and for querying)

`mcp__claude_ai_Linear__*` — good for search and read-heavy work
(`list_issues`, `get_issue`, `save_issue`). Note these are
interactively authenticated and **may be unavailable in headless or cron runs**, so
anything scripted should prefer the CLI.

## Statuses

The Product Engineering workflow:

| Status | Type |
|---|---|
| `Backlog` | backlog |
| `Todo` | unstarted |
| `In Progress` | started |
| `In Review` | started |
| `Done` | completed |
| `Canceled` | canceled |
| `Duplicate` | duplicate |

## How the skills should use this

- **`to-tickets`** — create one Linear issue per tracer-bullet ticket in team
  Product Engineering, starting in `Todo`. Express blocking edges as **native Linear
  blocked-by relations**, not as prose in the description and not as files under
  `.scratch/`. Any issue whose blockers are `Done` is grabbable.
- **`implement`** — take an issue by its `CIP-####` key. Move it to `In Progress` when
  work starts, and `In Review` when the PR opens. Leave the move to `Done` to the
  human merging.
- **`to-spec`** — link the resulting spec back to the originating issue as a comment or
  attachment on the Linear issue.
- **`code-review`** — the "Spec" axis reads the originating `CIP-####` issue for intent.

## Linking PRs to issues

Reference the issue key in the PR title or body (e.g. `CIP-3233`) so Linear's GitHub
integration attaches the PR to the issue automatically.

## PRs as a request surface

**Off.** External pull requests are not treated as incoming triage requests for this
repo. Flip this flag if that changes.
