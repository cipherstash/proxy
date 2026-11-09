---
status: accepted
---

# Release Proxy from main with a release-plz release PR

## Context

Releases of Proxy from main used to take two manual steps.
A person opened a prepare-release PR that bumped `[workspace.package] version` and renamed the `## [Unreleased]` section of `CHANGELOG.md`.
After it merged, they ran `mise run release vX.Y.Z` to tag the merged commit and create the GitHub release.
Publishing that release started `release.yml`, which builds the binaries and pushes the Docker images `X.Y.Z`, `X.Y` and `latest`.
Each step was a chance to slip, for example by tagging a commit before the version bump.

Other CipherStash Rust repositories use [release-plz](https://release-plz.dev).
Proxy differs from them in ways that rule out copying their setup as it is:

- Proxy ships a binary and a Docker image, and nothing is published to crates.io.
- `CHANGELOG.md` is written by hand for users, with compatibility notes that commit messages don't carry.
- The required `Build (Rust MSRV)` check comes from `test.yml`, and the Docker publish runs on `release: published`.
  Neither event fires for a PR or release created with the default `GITHUB_TOKEN`.
- Every commit on main must be signed.
- 2.2.x maintenance releases come from `release/2.2`, while main is on 3.x.

## Decision

Run release-plz on every push to main, in `.github/workflows/release-plz.yml`, configured by `release-plz.toml`.

- **Release PR.** release-plz keeps one PR open on a `release-plz-*` branch, bumping the workspace version from the conventional commits since the last release.
  It makes the commit through the GitHub API, so GitHub marks it Verified and it meets main's signed-commit rule.
  Merging that PR is the release, and the approval it needs is the human gate before production.
- **Versions from git tags.** `git_only = true` on `cipherstash-proxy` reads the last version from `v*` tags instead of a registry, and `publish = false`.
  release-plz takes the highest semver tag that matches, so 2.2.x tags never affect main's version.
  `git_only` is set on the package, not the workspace: workspace-wide, release-plz also looks up every other member at the last tag, and fails on members added since then (`cipherstash-proxy-burn-in` was added after v3.0.1).
- **Only `cipherstash-proxy` is released.** Workspace members that inherit `version.workspace` move with it.
- **No semver checks.** `semver_check = false`, because Proxy's library target is internal and checking it would turn refactors into major version bumps.
- **Hand-written changelog.** `changelog_update = false`.
  A person renames `## [Unreleased]` to `## [X.Y.Z] - <date>` in the release PR, and the `Release metadata` check fails any PR that changes the version without that heading.
- **Release only from the release PR.** `release_always = false`.
- **Tag format `vX.Y.Z`**, matching every existing tag, `release.yml` and the Docker tags.
- **GitHub App token.** The workflow mints a token from a GitHub App so the release PR runs `test.yml` and, once releases are automated, publishing a release starts `release.yml`.
  The App credentials and the tag signing key are secrets in a `release` environment that only main can use and that has no required reviewers, so the job never waits for approval.
- **Signed tags.** Release tags are annotated and GPG-signed.
- **Hardened workflow.** Actions are pinned to commit SHAs, the default permission is `contents: read`, and the concurrency group never cancels a run in progress.
- **release/2.2 stays manual.** Its version would be read from the highest tag (3.x), so release-plz can't run there.
  2.2.x releases keep the documented manual steps and `latest=false` on their Docker tags.

### Compared with cipherstash/stack

The same as stack: hand-written changelogs, release-plz creates the tags and GitHub releases, tags are annotated and GPG-signed, and merging a PR on main is the only human gate.

Different from stack, deliberately:

| | stack | Proxy | Why Proxy differs |
| --- | --- | --- | --- |
| Version bump | Written by hand in an ordinary PR | release-plz release PR | Automating the bump is the reason to adopt release-plz |
| Token | `GITHUB_TOKEN` | GitHub App token | The release PR must run the required check, and the release must start the Docker build |
| Secrets | Repository secrets | `release` environment, main only | Keeps the App key and signing key away from other workflows |
| Version source | crates.io | `v*` git tags | Nothing is published to a registry |
| Tag format | `<crate>-vX.Y.Z` | `vX.Y.Z` | Existing tags, `release.yml` and Docker tags depend on it |

### Rollout

The release PR comes first.
Until the release step is added to the same workflow, a person still tags the merged release PR with `mise run release`, which makes an unsigned lightweight tag.
The release step will create the signed tag and the GitHub release when a release PR merges, and then `mise run release` is retired for main.

## Consequences

- Approving the release PR approves shipping main as it is at that moment.
  Main keeps approvals when new commits land, and release-plz rebuilds the PR whenever main moves, so approve and merge in one sitting.
- When a person commits to the release PR (the changelog rename), release-plz closes it and opens a fresh one on the next push to main, so make that edit last.
- Switching on `changelog_update` to match other repositories would put generated entries beside the curated notes, so keep it off.
- If the App or its installation is removed, release PRs stop running CI and releases stop publishing Docker images, without any error from release-plz itself.
