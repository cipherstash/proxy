#!/usr/bin/env bash
# Checks that the workspace version in Cargo.toml has release notes in
# CHANGELOG.md. One script so that pull requests, the release workflow and
# `mise run release` apply the same rule.
#
# Usage (from the repository root):
#   release-metadata.sh version            print the workspace version
#   release-metadata.sh check [vX.Y.Z]     fail unless CHANGELOG.md has a
#                                          '## [X.Y.Z]' section for the
#                                          workspace version (and, with a tag,
#                                          unless the tag matches it)
#   release-metadata.sh check-pr BASE_REF  run `check` only if the workspace
#                                          version differs from BASE_REF's
#
# Exit status: 0 ok, 1 check failed, 2 usage or input error.
set -euo pipefail

error() {
  if [ -n "${GITHUB_ACTIONS:-}" ]; then
    echo "::error::$*" >&2
  else
    echo "error: $*" >&2
  fi
}

# Print the `version` under `[workspace.package]` of the Cargo.toml on stdin.
workspace_version() {
  awk '
    /^[[:space:]]*\[/ { in_pkg = ($0 ~ /^[[:space:]]*\[workspace\.package\][[:space:]]*(#.*)?$/); next }
    in_pkg && /^[[:space:]]*version[[:space:]]*=/ {
      sub(/^[[:space:]]*version[[:space:]]*=[[:space:]]*["\047]/, "")
      sub(/["\047].*$/, "")
      print
      exit
    }
  '
}

# True if CHANGELOG.md has a line starting with '## [VERSION]'.
has_changelog_section() {
  awk -v want="## [$1]" 'index($0, want) == 1 { found = 1; exit } END { exit !found }' CHANGELOG.md
}

cmd_version() {
  local version
  version="$(workspace_version <Cargo.toml)"
  if [ -z "$version" ]; then
    error "Cargo.toml has no version under [workspace.package]"
    exit 2
  fi
  echo "$version"
}

cmd_check() {
  local tag="${1:-}" version
  version="$(cmd_version)"

  if [ -n "$tag" ]; then
    if [[ ! "$tag" =~ ^v[0-9]+\.[0-9]+\.[0-9]+(-[0-9A-Za-z.]+)?$ ]]; then
      error "release tag must look like v1.2.3 (got '$tag')"
      exit 2
    fi
    if [ "${tag#v}" != "$version" ]; then
      error "Cargo.toml workspace version ($version) does not match release tag $tag. Bump the version before tagging."
      exit 1
    fi
  fi

  if [ ! -f CHANGELOG.md ]; then
    error "CHANGELOG.md not found in $(pwd)"
    exit 2
  fi
  if ! has_changelog_section "$version"; then
    error "CHANGELOG.md has no '## [$version]' section for the workspace version $version. In CHANGELOG.md, rename '## [Unreleased]' to '## [$version] - <release date>' (and add a new empty '## [Unreleased]' above it if you like)."
    exit 1
  fi

  echo "OK: Cargo.toml version $version has a '## [$version]' section in CHANGELOG.md${tag:+ and matches $tag}."
}

cmd_check_pr() {
  local base_ref="${1:?usage: release-metadata.sh check-pr BASE_REF}" base_toml base_version head_version
  if ! base_toml="$(git show "$base_ref:Cargo.toml" 2>/dev/null)"; then
    error "cannot read Cargo.toml at $base_ref (is it fetched?)"
    exit 2
  fi
  base_version="$(workspace_version <<<"$base_toml")"
  head_version="$(cmd_version)"

  if [ "$base_version" = "$head_version" ]; then
    echo "Workspace version unchanged ($head_version); nothing to check."
    return 0
  fi

  echo "Workspace version changes $base_version -> $head_version."
  cmd_check
}

case "${1:-}" in
  version) cmd_version ;;
  check) shift; cmd_check "$@" ;;
  check-pr) shift; cmd_check_pr "$@" ;;
  *)
    echo "usage: release-metadata.sh {version | check [vX.Y.Z] | check-pr BASE_REF}" >&2
    exit 2
    ;;
esac
