#!/usr/bin/env bash
#
# Soak test: drive sustained passthrough load against the running `proxy`
# container while sampling its cgroup memory, to reveal slow/monotonic growth.
# Runs several back-to-back rounds of simple-protocol inserts (max per-statement
# churn) and keeps sampling through a cooldown to see whether RSS is released.
#
# Usage: ./soak.sh <label> [rounds] [count_per_round] [workers]
#
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${HERE}/../.." && pwd)"
LABEL="${1:?usage: soak.sh <label> [rounds] [count] [workers]}"
ROUNDS="${2:-6}"
COUNT="${3:-50000}"
WORKERS="${4:-16}"
NET="$(docker inspect proxy --format '{{range $k,$v := .NetworkSettings.Networks}}{{$k}}{{end}}' | head -n1)"
# Cached extended protocol (pgx default) — reliable for the jsonb payload.
DSN="postgres://cipherstash:password@proxy:6432/cipherstash?sslmode=disable"
OUT="${HERE}/results/${LABEL}.csv"; mkdir -p "${HERE}/results"; echo "epoch_s,elapsed_s,mem_bytes" > "${OUT}"

# background sampler
( S=$(date +%s); while docker inspect -f '{{.State.Running}}' proxy 2>/dev/null | grep -q true; do
    m=$(docker exec proxy cat /sys/fs/cgroup/memory.current 2>/dev/null || echo 0)
    echo "$(date +%s),$(( $(date +%s)-S )),${m}" >> "${OUT}"; sleep 2; done ) &
SP=$!; trap 'kill "${SP}" 2>/dev/null || true' EXIT
sleep 3
echo "[$LABEL] baseline RSS: $(docker exec proxy cat /sys/fs/cgroup/memory.current|awk '{printf "%.1f",$1/1048576}') MiB"
for r in $(seq 1 "${ROUNDS}"); do
  docker run --rm --network "${NET}" -v "${HERE}:/src" -w /src -e GOFLAGS=-mod=mod golang:1.22-bookworm \
    bash -c "go mod tidy >/dev/null 2>&1; go run ./loadgen -count ${COUNT} -workers ${WORKERS} -dsn '${DSN}'" >/dev/null 2>&1 || true
  echo "[$LABEL] round ${r}/${ROUNDS} done; RSS=$(docker exec proxy cat /sys/fs/cgroup/memory.current|awk '{printf "%.1f",$1/1048576}') MiB"
done
echo "[$LABEL] cooldown 45s..."; sleep 45
kill "${SP}" 2>/dev/null || true; trap - EXIT
awk -F, 'NR>1{if($3>mx)mx=$3; if(mn==0||$3<mn)mn=$3; l=$3; if(NR==2)f=$3} END{printf "[%s] baseline=%.1f  peak=%.1f  final=%.1f  (min=%.1f) MiB\n","'"$LABEL"'",f/1048576,mx/1048576,l/1048576,mn/1048576}' "${OUT}"