#!/usr/bin/env bash
# Tests for release-metadata.sh. Run with: mise run test:release-metadata
set -uo pipefail

SCRIPT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)/release-metadata.sh"
failures=0
tests=0

# Keep the developer's own git config (hooks, templates, signing) out of the
# throwaway repositories.
export GIT_CONFIG_GLOBAL=/dev/null GIT_CONFIG_NOSYSTEM=1

dirs=()
trap 'rm -rf "${dirs[@]}"' EXIT

# Each test runs in its own git repository so `check-pr` can compare refs.
# Sets $d rather than printing it, so the trap above sees every directory.
new_repo() {
  d="$(mktemp -d)"
  dirs+=("$d")
  git -C "$d" init --quiet --initial-branch=main
  git -C "$d" config user.email test@example.com
  git -C "$d" config user.name test
}

write_cargo() { # dir version
  cat >"$1/Cargo.toml" <<EOF
[workspace]
members = ["packages/*"]

[workspace.package]
version = "$2"
edition = "2021"

[workspace.dependencies]
sqltk = { version = "0.11.0" }
EOF
}

write_changelog() { # dir heading...
  local dir="$1"
  shift
  {
    echo "# Changelog"
    echo
    for heading in "$@"; do
      echo "$heading"
      echo
      echo "- something"
      echo
    done
    echo "[3.0.1]: https://example.com/compare/v3.0.0...v3.0.1"
  } >"$dir/CHANGELOG.md"
}

commit_all() { # dir message
  git -C "$1" add -A
  git -C "$1" commit --quiet -m "$2"
}

# expect <name> <expected-exit> <expected-output-substring> <dir> <args...>
expect() {
  local name="$1" want_status="$2" want_text="$3" dir="$4"
  shift 4
  tests=$((tests + 1))
  local out status
  out="$(cd "$dir" && env -u GITHUB_ACTIONS bash "$SCRIPT" "$@" 2>&1)"
  status=$?
  if [ "$status" -ne "$want_status" ] || [[ "$out" != *"$want_text"* ]]; then
    failures=$((failures + 1))
    echo "FAIL: $name"
    echo "  wanted exit $want_status and output containing: $want_text"
    echo "  got exit $status and output:"
    printf '    %s\n' "$out"
  else
    echo "ok: $name"
  fi
}

# --- version -------------------------------------------------------------

new_repo
write_cargo "$d" "3.1.0"
expect "version prints the workspace version, not a dependency version" 0 "3.1.0" "$d" version

new_repo
cat >"$d/Cargo.toml" <<'EOF2'
[workspace.package] # shared by every package
version = '3.2.0' # bumped by release-plz
EOF2
expect "version handles single quotes and trailing comments" 0 "3.2.0" "$d" version

# --- check ---------------------------------------------------------------

new_repo
write_cargo "$d" "3.1.0"
write_changelog "$d" "## [Unreleased]" "## [3.0.1] - 2026-08-05"
expect "check fails when the version has no changelog section" 1 "rename '## [Unreleased]' to '## [3.1.0] - <release date>'" "$d" check

new_repo
write_cargo "$d" "3.1.0"
write_changelog "$d" "## [3.1.0] - 2026-11-09" "## [3.0.1] - 2026-08-05"
expect "check passes when the changelog has the section" 0 "3.1.0" "$d" check

new_repo
write_cargo "$d" "3.0.10"
write_changelog "$d" "## [Unreleased]" "## [3.0.1] - 2026-08-05"
expect "check does not treat 3.0.1 as a section for 3.0.10" 1 "## [3.0.10]" "$d" check

new_repo
write_cargo "$d" "3.0.1"
write_changelog "$d" "## [Unreleased]" "Not a heading: ## [3.0.1]"
expect "check only accepts the section as a heading at the start of a line" 1 "## [3.0.1]" "$d" check

new_repo
write_cargo "$d" "3.1.0"
expect "check reports a missing CHANGELOG.md as an input error" 2 "CHANGELOG.md not found" "$d" check

new_repo
write_cargo "$d" "3.1.0"
write_changelog "$d" "## [3.1.0] - 2026-11-09"
expect "check with a matching tag passes" 0 "v3.1.0" "$d" check v3.1.0
expect "check with a mismatched tag fails" 1 "does not match release tag v3.2.0" "$d" check v3.2.0
expect "check rejects a malformed tag" 2 "must look like v1.2.3" "$d" check 3.1

# --- check-pr --------------------------------------------------------------

new_repo
write_cargo "$d" "3.0.1"
write_changelog "$d" "## [Unreleased]" "## [3.0.1] - 2026-08-05"
commit_all "$d" base
git -C "$d" branch base
echo "// change" >"$d/lib.rs"
commit_all "$d" "feat: unrelated change"
expect "check-pr skips a PR that does not change the version" 0 "unchanged" "$d" check-pr base

write_cargo "$d" "3.1.0"
commit_all "$d" "chore: release v3.1.0"
expect "check-pr fails a version bump without a changelog section" 1 "rename '## [Unreleased]'" "$d" check-pr base

write_changelog "$d" "## [3.1.0] - 2026-11-09" "## [3.0.1] - 2026-08-05"
commit_all "$d" "docs: changelog for 3.1.0"
expect "check-pr passes once the section exists" 0 "3.0.1 -> 3.1.0" "$d" check-pr base

expect "check-pr reports an unknown base ref" 2 "cannot read Cargo.toml at no-such-ref" "$d" check-pr no-such-ref

# --- GitHub annotations ----------------------------------------------------

new_repo
write_cargo "$d" "3.1.0"
write_changelog "$d" "## [Unreleased]"
tests=$((tests + 1))
out="$(cd "$d" && GITHUB_ACTIONS=true bash "$SCRIPT" check 2>&1)"
if [[ "$out" == "::error::"* ]]; then
  echo "ok: errors are GitHub annotations inside Actions"
else
  failures=$((failures + 1))
  echo "FAIL: errors are GitHub annotations inside Actions"
  printf '    %s\n' "$out"
fi

echo
echo "$((tests - failures))/$tests passed"
[ "$failures" -eq 0 ]
