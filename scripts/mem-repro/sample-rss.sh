#!/usr/bin/env bash
#
# Sample the resident set size (RSS) of a running process over time and append
# it to a CSV. Works on Linux and macOS (`ps` reports RSS/VSZ in KiB on both).
#
# Usage:
#   ./sample-rss.sh <process-name-pattern> <output.csv> [interval_seconds]
#
# Example:
#   ./sample-rss.sh cipherstash-proxy results/glibc-baseline.csv 1
#
# Stop with Ctrl-C (or send SIGTERM). The CSV columns are:
#   epoch_s,elapsed_s,pid,rss_kb,vsz_kb
#
set -euo pipefail

PATTERN="${1:?usage: sample-rss.sh <process-pattern> <output.csv> [interval]}"
OUT="${2:?usage: sample-rss.sh <process-pattern> <output.csv> [interval]}"
INTERVAL="${3:-1}"

mkdir -p "$(dirname "$OUT")"
echo "epoch_s,elapsed_s,pid,rss_kb,vsz_kb" > "$OUT"

# Resolve the PID once so we follow a single process for the whole run.
PID="$(pgrep -f "$PATTERN" | head -n1 || true)"
if [[ -z "${PID}" ]]; then
  echo "No process matching '${PATTERN}' is running. Start the proxy first." >&2
  exit 1
fi

echo "Sampling RSS of pid ${PID} (pattern '${PATTERN}') every ${INTERVAL}s -> ${OUT}" >&2
START="$(date +%s)"

peak=0
while kill -0 "$PID" 2>/dev/null; do
  now="$(date +%s)"
  elapsed=$(( now - START ))
  # rss= and vsz= suppress headers; values are in KiB.
  read -r rss vsz < <(ps -o rss=,vsz= -p "$PID" 2>/dev/null || echo "0 0")
  rss="${rss:-0}"; vsz="${vsz:-0}"
  echo "${now},${elapsed},${PID},${rss},${vsz}" >> "$OUT"
  if (( rss > peak )); then peak=$rss; fi
  sleep "$INTERVAL"
done

echo "Process ${PID} exited. Peak RSS: $(( peak / 1024 )) MiB. Samples in ${OUT}" >&2
