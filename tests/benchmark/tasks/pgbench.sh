#!/usr/bin/env bash
set -e

#MISE description="Run pgbench"
#USAGE flag "-c --clients <clients>" help="Number of clients to run"
#USAGE flag "-t --time <time>" help="Time in seconds to run test"
#USAGE flag "-t --output <output>" help="Output file"
#USAGE flag "-p --port <port>" help="Port of target service"
#USAGE flag "-h --host <host>" help="Port of target service"
#USAGE flag "-P --protocol <protocol>" default="simple" help="Procol to use" {
#USAGE  choices "simple" "extended" "prepared"
#USAGE }
#USAGE flag "--transaction <transaction>" default="default" help="Procol to use" {
#USAGE  choices "default" "plaintext" "encrypted"
#USAGE }


echo "pgbench --protocol=$usage_protocol --file=/etc/postgresql/benchmark/sql/transaction-$usage_transaction.sql --jobs=2 --time=${usage_time} --client=${usage_clients} --host=${usage_host} --port=${usage_port} --no-vacuum --report-per-command"

OUTPUT="$(docker compose run --rm postgres${CONTAINER_SUFFIX:-} pgbench --protocol=$usage_protocol --file=/etc/postgresql/benchmark/sql/transaction-$usage_transaction.sql --jobs=2 --time=${usage_time} --client=${usage_clients} --host=${usage_host} --port=${usage_port} --no-vacuum --report-per-command)"


latency=$(echo "$OUTPUT" | grep "latency average = " | awk '{print $4}')
init_conn_time=$(echo "$OUTPUT" | grep "initial connection time = " | awk '{print $5}')
tps=$(echo "$OUTPUT" | grep "tps = " | awk '{print $3}')

echo "$usage_clients,$latency,$init_conn_time,$tps" >> $usage_output
