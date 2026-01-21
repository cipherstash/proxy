#!/usr/bin/env bash
#MISE description="Run a single k6 benchmark script"
#USAGE flag "--script <script>" help="Script name (without .js)" {
#USAGE   choices "jsonb-ste-vec-insert" "jsonb-ste-vec-containment" "jsonb-ste-vec-large-payload" "jsonb-large-payload" "text-equality"
#USAGE }
#USAGE flag "--target <target>" default="proxy" help="Target service" {
#USAGE   choices "postgres" "proxy"
#USAGE }
#USAGE flag "--vus <vus>" default="10" help="Number of virtual users"
#USAGE flag "--duration <duration>" default="30s" help="Test duration"

set -e

# Use --network=host on Linux for direct port access (CI)
# On macOS, host.docker.internal is used instead
if [ "$(uname)" = "Linux" ]; then
  NETWORK_FLAG="--network=host"
  DEFAULT_DB_HOST="127.0.0.1"
else
  NETWORK_FLAG=""
  DEFAULT_DB_HOST="host.docker.internal"
fi

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

docker run --rm $NETWORK_FLAG \
  -v "$SCRIPT_DIR/k6/scripts:/scripts" \
  -v "$SCRIPT_DIR/results/k6:/scripts/results/k6" \
  -e K6_TARGET=${usage_target} \
  -e K6_VUS=${usage_vus} \
  -e K6_DURATION=${usage_duration} \
  -e K6_DB_HOST=${K6_DB_HOST:-$DEFAULT_DB_HOST} \
  k6-pgxpool run /scripts/${usage_script}.js
