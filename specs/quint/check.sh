#!/usr/bin/env bash
# Runs the Quint checks for the routing model. See README.md for what each
# stage does and does not show.
#
#   ./check.sh            typecheck, scenario tests, simulation
#   ./check.sh tlc        exhaustive TLC checks, expected findings included
#   ./check.sh mutations  each injected fault must be caught by TLC
#   ./check.sh all        everything
set -euo pipefail

cd "$(dirname "$0")"

QUINT=(npx --no-install quint)
MODEL=routing/routing.qnt
TESTS=routing/routing_test.qnt
SEED=42

fail() {
  echo "FAIL: $*" >&2
  exit 1
}

# verify_holds INSTANCE INVARIANT [FILE]: TLC must explore every state and find
# no violation.
verify_holds() {
  local main=$1 invariant=$2 file=${3:-$MODEL} out
  echo "== TLC: $main satisfies $invariant"
  if ! out=$("${QUINT[@]}" verify "$file" --main "$main" --backend=tlc --invariant "$invariant" 2>&1); then
    echo "$out" >&2
    fail "$main should satisfy $invariant"
  fi
  # The summary is informational: a change in Quint's wording must not fail
  # a check that passed.
  grep -E "distinct states found|No violation found" <<<"$out" || true
}

# verify_violated INSTANCE INVARIANT [FILE]: TLC must report a counterexample,
# not an error of any other kind.
verify_violated() {
  local main=$1 invariant=$2 file=${3:-$MODEL} out
  echo "== TLC: $main violates $invariant"
  if out=$("${QUINT[@]}" verify "$file" --main "$main" --backend=tlc --invariant "$invariant" 2>&1); then
    echo "$out" >&2
    fail "$main should violate $invariant"
  fi
  grep -q "\[violation\]" <<<"$out" || {
    echo "$out" >&2
    fail "$main did not report a violation of $invariant"
  }
  echo "counterexample found, as expected"
}

# simulate INSTANCE WITNESS...: sample traces, require no violation of safety,
# and require every witness to be reached in at least one trace. A witness
# reached in 0 traces means the state is dead, so the model is over-constrained.
simulate() {
  local main=$1 out
  shift
  echo "== simulation: $main"
  if ! out=$("${QUINT[@]}" run "$MODEL" --main "$main" --backend typescript \
    --max-samples=2000 --max-steps=12 --seed="$SEED" --invariant safety \
    --witnesses "$@" 2>&1); then
    echo "$out" >&2
    fail "$main should satisfy safety in simulation"
  fi
  grep -E "was witnessed in" <<<"$out"
  if grep -qE "was witnessed in 0 trace" <<<"$out"; then
    fail "$main: a witness was never reached"
  fi
}

basic() {
  echo "== typecheck"
  "${QUINT[@]}" typecheck "$MODEL"
  "${QUINT[@]}" typecheck "$TESTS"

  echo "== scenario tests"
  for main in routingAsBuiltTest routingDomainCheckTest; do
    "${QUINT[@]}" test "$TESTS" --main "$main" --backend typescript --seed "$SEED"
  done

  echo "== simulation"
  simulate asBuilt ciphertextStored readFellBack statementRefused batchRuleFired \
    unknownBindRefused settingsChangedRefused unsafePlaintextStored \
    statementPrepared inTransaction
  simulate concurrentDdl ciphertextStored dbRejected txSnapshotStale \
    preparedRouteStale
}

tlc() {
  # Properties that must hold.
  verify_holds asBuilt safety
  verify_holds concurrentDdl safety

  # Findings: each counterexample must still exist. When one stops existing,
  # the behaviour changed; update requirements.md and this list together.
  verify_violated concurrentDdl noPlaintextToCurrentlyEncrypted       # F1
  verify_violated concurrentDdlNoDomainCheck noPlaintextAtRest        # F1
  verify_violated uncheckedWrites noPlaintextToKnownEncrypted         # F2
}

# mutate NAME FROM TO INVARIANT: replace FROM with TO in a copy of the model
# and require TLC to find a violation of INVARIANT in the asBuilt
# instance.
mutate() {
  local name=$1 from=$2 to=$3 invariant=$4 dir file
  dir=$(mktemp -d)
  file="$dir/routing.qnt"
  cp "$MODEL" "$file"
  grep -qF -- "$from" "$file" || fail "mutation $name: pattern not found"
  FROM=$from TO=$to perl -0pi -e 's/\Q$ENV{FROM}\E/$ENV{TO}/' "$file"
  echo "== mutation $name"
  verify_violated asBuilt "$invariant" "$file"
  rm -rf "$dir"
}

mutations() {
  # M1: a write may fall back while encrypted columns are configured.
  mutate M1-write-falls-back \
    "not(mapErrs) and not(writesEncrypted)" \
    "not(mapErrs)" \
    noPlaintextToKnownEncrypted
  # M2: a table Proxy cannot resolve type checks as native (#486).
  mutate M2-unresolvable-is-native \
    "val typeCheckFails = s.kind == Gap or v == VUnknown" \
    "val typeCheckFails = s.kind == Gap" \
    noPlaintextToUnresolvable
  # M3: no batch rule, so a fallback carries a mapped statement as written.
  mutate M3-no-batch-rule \
    "else if (batchRefused(routes))" \
    "else if (false)" \
    noPlaintextToKnownEncrypted
  # M4: Bind ignores a change to the mapping setting since Parse.
  mutate M4-no-bind-settings-check \
    "val settingsChanged = p.stmt.kind != Unchecked and conn.unsafe != p.unsafeAtParse" \
    "val settingsChanged = false" \
    noPlaintextToKnownEncrypted
}

case "${1:-basic}" in
  basic) basic ;;
  tlc) tlc ;;
  mutations) mutations ;;
  all) basic; tlc; mutations ;;
  *) fail "unknown stage: $1" ;;
esac
