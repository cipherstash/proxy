#!/usr/bin/env bash
#
# Containerised variant of run.sh: drives the Go load generator from a `golang`
# container against the proxy running in the `proxy` container, while sampling
# the proxy container's cgroup memory (the k8s-relevant number).
#
# Prereqs:
#   - `proxy` and `postgres` containers running (mise run proxy:up).
#   - schema.sql applied to postgres.
#
# Usage:
#   ./run-docker.sh <label> [count] [workers]
#
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${HERE}/../.." && pwd)"

LABEL="${1:?usage: run-docker.sh <label> [count] [workers]}"
COUNT="${2:-20000}"
WORKERS="${3:-16}"

# DB creds for the proxy connection (proxy listens on 6432 inside its network).
PGUSER="${CS_DATABASE__USERNAME:-cipherstash}"
PGPASS="${CS_DATABASE__PASSWORD_ESCAPED_FOR_TESTS:-p%40ssword}"
PGDB="${CS_DATABASE__NAME:-cipherstash}"
DSN="postgres://${PGUSER}:${PGPASS}@proxy:6432/${PGDB}?sslmode=disable"

# Resolve the docker network the proxy is attached to.
NET="$(docker inspect proxy --format '{{range $k,$v := .NetworkSettings.Networks}}{{$k}}{{end}}' | head -n1)"
OUT="${HERE}/results/${LABEL}.csv"
mkdir -p "${HERE}/results"
echo "epoch_s,elapsed_s,mem_bytes" > "${OUT}"

echo "== Run '${LABEL}': ${COUNT} inserts, ${WORKERS} workers (network: ${NET}) =="

# Background cgroup sampler: memory.current is what kubelet watches for OOM.
( START=$(date +%s)
  while docker inspect -f '{{.State.Running}}' proxy 2>/dev/null | grep -q true; do
    now=$(date +%s)
    mem=$(docker exec proxy cat /sys/fs/cgroup/memory.current 2>/dev/null || echo 0)
    echo "${now},$(( now - START )),${mem}" >> "${OUT}"
    sleep 1
  done ) &
SAMPLER_PID=$!
trap 'kill "${SAMPLER_PID}" 2>/dev/null || true' EXIT

sleep 3  # baseline

# Run the load generator in a golang container on the proxy's network.
docker run --rm --network "${NET}" \
  -v "${HERE}:/src" -w /src \
  -e GOFLAGS=-mod=mod \
  golang:1.22-bookworm \
  bash -c "go mod tidy >/dev/null 2>&1; go run ./loadgen -count ${COUNT} -workers ${WORKERS} -dsn '${DSN}'"

echo "Load complete. Cooldown 60s to observe memory release..."
sleep 60

kill "${SAMPLER_PID}" 2>/dev/null || true
trap - EXIT

PEAK=$(tail -n +2 "${OUT}" | awk -F, 'BEGIN{m=0}{if($3>m)m=$3}END{print m}')
LAST=$(tail -n1 "${OUT}" | awk -F, '{print $3}')
awk -v p="$PEAK" -v l="$LAST" 'BEGIN{printf "Peak: %.1f MiB | Post-cooldown: %.1f MiB\n", p/1048576, l/1048576}'
echo "Timeseries: ${OUT}"
