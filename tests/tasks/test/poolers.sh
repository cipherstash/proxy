#!/usr/bin/env bash
#MISE description="Connection pooler compatibility matrix (PgBouncer, PgCat) in front of and behind Proxy"
#
# Starts PostgreSQL, Proxy (with and without a default keyset), PgBouncer
# 1.19 / 1.24 (max_prepared_statements off and on), PgCat 1.2 (prepared
# statement cache off and on) and one Proxy in front of each pooler, then runs
# the matrix in packages/cipherstash-proxy-integration/src/pooler.
#
# Requires Docker and CipherStash credentials (see DEVELOPMENT.md). The
# multitenant checks switch to CS_DEFAULT_KEYSET_ID with
# `SET CIPHERSTASH.KEYSET_ID` on a Proxy that has no default keyset.
#
# Environment:
#   POOLERS_SKIP_BUILD=1     reuse the existing cipherstash/proxy:latest image
#   POOLERS_KEEP_RUNNING=1   leave the matrix containers running afterwards
#   POOLERS_TEST_FILTER=...  extra nextest filter, e.g. 'test(pgbouncer_1_19::)'
#   POOLERS_GOLANG=supported  default: require the full Go suite on eight supported cells
#   POOLERS_GOLANG=informational  explicitly inspect all cells; not a supported gate
#   POOLERS_GOLANG=off        local Rust-only diagnosis; forbidden in CI
#   POOLERS_RESULTS_DIR=...   persistent artifacts (default target/pooler-results)
# Supported gates reject partial filters and reused Proxy images. CI requires supported mode.
#
# Every cell prints a `MATRIX | ...` line; the summary at the end collects them.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
cd "${root}"

mode="${POOLERS_GOLANG:-supported}"
case "${mode}" in
  1) mode=informational ;;
  supported|informational|off) ;;
  *) echo "POOLERS_GOLANG must be supported, informational, or off" >&2; exit 1 ;;
esac
if [ -n "${CI:-}" ] && [ "${mode}" != supported ]; then
  echo 'CI requires POOLERS_GOLANG=supported' >&2
  exit 1
fi
if [ "${mode}" = supported ]; then
  if [ -n "${POOLERS_TEST_FILTER:-}" ] || [ -n "${POOLERS_SKIP_BUILD:-}" ]; then
    echo 'Supported gate requires the complete Rust matrix and Proxy built from current source; remove POOLERS_TEST_FILTER and POOLERS_SKIP_BUILD' >&2
    exit 1
  fi
fi
for required in CS_WORKSPACE_CRN CS_CLIENT_ID CS_CLIENT_KEY CS_CLIENT_ACCESS_KEY CS_DEFAULT_KEYSET_ID; do
  if [ -z "${!required:-}" ]; then
    echo "${required} must be set to run the pooler matrix" >&2
    exit 1
  fi
done
results="${POOLERS_RESULTS_DIR:-${root}/target/pooler-results}"
mkdir -p "${results}"
results="$(cd "${results}" && pwd)"
log="${results}/matrix.log"
: > "${log}"

compose=(docker compose -f docker-compose.yml -f docker-compose.poolers.yml)
proxies=(proxy proxy-mt proxy-via-pgbouncer-1-19 proxy-via-pgbouncer-1-24 proxy-via-pgbouncer-1-24-ps proxy-via-pgcat-1-2 proxy-via-pgcat-1-2-ps)
poolers=(pgbouncer-1-19 pgbouncer-1-24 pgbouncer-1-24-ps pgcat-1-2 pgcat-1-2-ps)
ports=(6432 6442 6451 6452 6453 6454 6455 6461 6462 6463 6464 6465)

# Proxies in the matrix talk plaintext to PostgreSQL inside the compose network.
export CS_DATABASE__HOST=postgres
export CS_DATABASE__PORT=5532

teardown() {
  (cd tests && "${compose[@]}" logs --no-color "${proxies[@]}" "${poolers[@]}" >"${results}/services.log" 2>&1) || true
  if [ -z "${POOLERS_KEEP_RUNNING:-}" ]; then
    (cd tests && "${compose[@]}" rm --stop --force "${proxies[@]}" "${poolers[@]}" >/dev/null 2>&1) || true
  fi
}
trap teardown EXIT

echo '###############################################'
echo '# PostgreSQL'
echo '###############################################'
mise run postgres:up postgres --extra-args "--detach --wait"
# Run from tests/ so tests/mise.tcp.toml wins over any local database settings.
(cd tests && mise --env tcp run postgres:setup)

if [ -z "${POOLERS_SKIP_BUILD:-}" ]; then
  echo '###############################################'
  echo '# Build Proxy from source'
  echo '###############################################'
  arch="$(uname -m | sed -e 's/arm64/aarch64/' -e 's/x64/x86_64/')"
  platform="linux/$(uname -m | sed -e 's/x86_64/amd64/' -e 's/aarch64/arm64/')"
  mise run build:binary --target "${arch}-unknown-linux-gnu"
  mise run build:docker --platform "${platform}"
fi

echo '###############################################'
echo '# Proxies and poolers'
echo '###############################################'
(
  cd tests
  "${compose[@]}" rm --stop --force "${proxies[@]}" "${poolers[@]}" >/dev/null 2>&1 || true
  "${compose[@]}" up --build --detach --wait "${poolers[@]}" "${proxies[@]}"
)

for port in "${ports[@]}"; do
  case "${port}" in
    # These upstream poolers reject 3.2 startup. Dedicated Rust probes require
    # the exact rejection; psql through Proxy cannot establish readiness here.
    6451|6454|6455) continue ;;
  esac
  for attempt in $(seq 1 30); do
    if PGPASSWORD="${CS_DATABASE__PASSWORD}" PGCONNECT_TIMEOUT=2 psql -h localhost -p "${port}" \
      -U "${CS_DATABASE__USERNAME}" -d "$([ "${port}" -ge 6451 ] && echo pg_session || echo cipherstash)" \
      -c 'SELECT 1' >/dev/null 2>&1; then
      echo "ready: localhost:${port}"
      break
    fi
    if [ "${attempt}" -eq 30 ]; then
      # Continue collecting diagnostics; typed matrix probes will fail these
      # endpoints as infrastructure errors, never expected incompatibilities.
      echo "warning: localhost:${port} did not accept connections; its cells will fail"
      break
    fi
    sleep 1
  done
done

echo '###############################################'
echo '# Matrix'
echo '###############################################'
# Runner and cleanup fixtures deliberately report incompatible/error observations.
# Keep their output separate from the actual deployment matrix evidence.
if ! cargo nextest run --features cipherstash-proxy-integration/pooler-tests \
  -E 'package(cipherstash-proxy-integration) and (test(pooler::runner_tests::) or test(pooler::checks::cleanup_regression_tests::))' \
  >"${results}/runner-tests.log" 2>&1; then
  cat "${results}/runner-tests.log"
  exit 1
fi
filter='package(cipherstash-proxy-integration) and test(pooler::matrix::)'
if [ -n "${POOLERS_TEST_FILTER:-}" ]; then
  filter="${filter} and ${POOLERS_TEST_FILTER}"
fi

set +e
CS_TEST_POOLER_KEYSET_ID="${CS_DEFAULT_KEYSET_ID}" \
  cargo nextest run --no-fail-fast --nocapture --test-threads 1 \
  --features cipherstash-proxy-integration/pooler-tests \
  -E "${filter}" 2>&1 | tee "${log}"
status=${PIPESTATUS[0]}
set -e

if [ "${mode}" != off ]; then
  echo '###############################################'
  echo "# Go pgx suite (${mode})"
  echo '###############################################'
  network="$(docker network ls --filter name=tests_postgres --quiet)"
  if [ -z "${network}" ] || [[ "${network}" == *$'\n'* ]]; then
    echo 'Expected exactly one tests_postgres Docker network' >&2
    exit 1
  fi
  docker build --quiet tests/integration/golang --tag cipherstash/proxy/test-integration-golang >/dev/null
  # Session pooling supports the complete driver suite. Transaction pooling
  # remains outside this full-suite claim; the Rust matrix gates its applicable
  # statement and isolation behaviors independently.
  cells=("proxy:6432/cipherstash direct")
  for pooler in pgbouncer-1-19 pgbouncer-1-24 pgbouncer-1-24-ps pgcat-1-2 pgcat-1-2-ps; do
    cells+=("${pooler}:6432/app_session ${pooler}-pooler-in-front-session")
  done
  for pooler in pgbouncer-1-24 pgbouncer-1-24-ps; do
    cells+=("proxy-via-${pooler}:6432/pg_app_session ${pooler}-proxy-in-front-session")
  done
  if [ "${mode}" = informational ]; then
    for pooler in pgbouncer-1-19 pgbouncer-1-24 pgbouncer-1-24-ps pgcat-1-2 pgcat-1-2-ps; do
      cells+=("${pooler}:6432/app_txn ${pooler}-pooler-in-front-transaction")
      cells+=("proxy-via-${pooler}:6432/pg_app_txn ${pooler}-proxy-in-front-transaction")
    done
    for pooler in pgbouncer-1-19 pgcat-1-2 pgcat-1-2-ps; do
      cells+=("proxy-via-${pooler}:6432/pg_app_session ${pooler}-proxy-in-front-session")
    done
  fi
  baseline="${results}/go-direct-manifest.json"
  : > "${baseline}"
  baseline_ready=0
  for cell in "${cells[@]}"; do
    read -r target label <<<"${cell}"
    # The suite expects `encrypted` to hold only its own rows. A failed setup is
    # fatal even in informational mode, since there is then no valid probe.
    docker exec postgres psql -v ON_ERROR_STOP=1 -q "postgresql://${CS_DATABASE__USERNAME}:${CS_DATABASE__PASSWORD_ESCAPED_FOR_TESTS}@localhost:5532/${CS_DATABASE__NAME}" \
      -c 'TRUNCATE encrypted' </dev/null >/dev/null
    events="${results}/go-${label}.jsonl"
    set +e
    docker run --rm --network "${network}" \
      -e DATABASE_URL="postgresql://${CS_DATABASE__USERNAME}:${CS_DATABASE__PASSWORD_ESCAPED_FOR_TESTS}@${target}" \
      cipherstash/proxy/test-integration-golang \
      go test -json -count=1 -parallel 32 ./... >"${events}" 2>"${results}/go-${label}.stderr.log" </dev/null
    driver_status=$?
    set -e
    checker=(python3 tests/tasks/test/pooler_go_result.py "${events}" --exit-code "${driver_status}")
    if [ "${label}" = direct ]; then
      checker+=(--manifest "${baseline}")
    elif [ "${baseline_ready}" -eq 1 ]; then
      checker+=(--baseline "${baseline}")
    else
      echo "GOLANG | ${label} | INCONCLUSIVE: direct baseline failed | ${mode}" | tee -a "${log}"
      status=1
      continue
    fi
    if summary="$("${checker[@]}" 2>&1)"; then
      echo "GOLANG | ${label} | PASS: ${summary} | ${mode}" | tee -a "${log}"
      if [ "${label}" = direct ]; then baseline_ready=1; fi
    else
      echo "GOLANG | ${label} | FAIL: ${summary} | ${mode}" | tee -a "${log}"
      if [ "${mode}" = supported ] || [ "${label}" = direct ]; then status=1; fi
    fi
  done
fi

echo
echo '###############################################'
echo '# Observed matrix'
echo '###############################################'
grep -E '^(MATRIX|GOLANG)' "${log}" | sort || true
echo "Artifacts: ${results}"

exit "${status}"
