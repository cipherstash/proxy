#!/usr/bin/env bash
#
# Drive one memory-reproduction run against an already-running proxy and record
# its RSS for the duration of the load.
#
# Prereqs (do these once, per the README):
#   - Proxy is already running (built in the variant you want to measure).
#   - Postgres is up and `schema.sql` has been applied.
#   - Go is installed (for the load generator at the repo root).
#
# Usage:
#   ./run.sh <label> [count] [workers] [dsn]
#
# Example:
#   ./run.sh glibc-baseline 20000 16 "postgres://user:pass@127.0.0.1:6432/db?sslmode=disable"
#
# Output:
#   results/<label>.csv  -- RSS timeseries (see sample-rss.sh)
#
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${HERE}/../.." && pwd)"

LABEL="${1:?usage: run.sh <label> [count] [workers] [dsn]}"
COUNT="${2:-20000}"
WORKERS="${3:-16}"
DSN="${4:-postgres://bloomuser:password@127.0.0.1:6432/pdts_db?sslmode=disable}"

PROXY_PATTERN="cipherstash-proxy"
OUT="${HERE}/results/${LABEL}.csv"

if ! pgrep -f "${PROXY_PATTERN}" >/dev/null; then
  echo "Proxy ('${PROXY_PATTERN}') is not running. Start it first (see README)." >&2
  exit 1
fi

echo "== Run '${LABEL}': ${COUNT} inserts, ${WORKERS} workers =="

# Start the RSS sampler in the background.
"${HERE}/sample-rss.sh" "${PROXY_PATTERN}" "${OUT}" 1 &
SAMPLER_PID=$!
# Always stop the sampler, even on error/interrupt.
trap 'kill "${SAMPLER_PID}" 2>/dev/null || true' EXIT

# Give the sampler a moment to capture a pre-load baseline.
sleep 3

# Run the load generator (Go module + loadgen package live alongside this script).
( cd "${HERE}" && go run ./loadgen -count "${COUNT}" -workers "${WORKERS}" -dsn "${DSN}" )

# Capture a short cooldown tail so we can see whether RSS is released after the
# load stops (the key signal: glibc tends to hold it; jemalloc releases it).
echo "Load complete. Sampling cooldown for 60s to observe memory release..."
sleep 60

kill "${SAMPLER_PID}" 2>/dev/null || true
trap - EXIT

PEAK_KB="$(tail -n +2 "${OUT}" | awk -F, 'BEGIN{m=0} {if($4>m)m=$4} END{print m}')"
LAST_KB="$(tail -n1 "${OUT}" | awk -F, '{print $4}')"
echo "Done. Peak RSS: $(( PEAK_KB / 1024 )) MiB | Post-cooldown RSS: $(( LAST_KB / 1024 )) MiB"
echo "Timeseries: ${OUT}"
