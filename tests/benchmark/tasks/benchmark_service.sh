#!/usr/bin/env bash
#MISE description="Run pgbench against a single target"
#USAGE flag "--host <host>" default="host.docker.internal" help="Host of target service"
#USAGE flag "--port <port>" help="Port of target service"
#USAGE flag "--time <time>" help="Time for each run"
#USAGE flag "--target <target>" help="Target service" {
#USAGE  choices "postgres" "proxy" "pgbouncer"
#USAGE }
#USAGE flag "--protocol <protocol>" default="simple" help="Procol to use" {
#USAGE  choices "simple" "extended" "prepared"
#USAGE }
#USAGE flag "--transaction <transaction>" default="default" help="Procol to use" {
#USAGE  choices "default" "plaintext" "encrypted"
#USAGE }
#!/bin/bash

set -e

# clients_array=(10)
clients_array=(5 10 50 75 100 125)


# output="results/$usage_target-$(date +"%Y%m%d%H%M").csv"
output="results/$usage_target-$usage_protocol-$usage_transaction.csv"

# CSV header
echo "clients,latency,init_conn_time,tps" > $output

for clients in "${clients_array[@]}" ; do
    echo "Benchmark {clients: $clients, target: $usage_target, protocol: $usage_protocol, transaction: $usage_transaction}"
    mise run pgbench --host=$usage_host --port=$usage_port --transaction=$usage_transaction --protocol=$usage_protocol --clients $clients --time $usage_time --output $output
    sleep 2
done


