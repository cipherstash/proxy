#!/usr/bin/env bash
#MISE description="Test Prometheus metrics are exported and updated"

#!/bin/bash

set -e

source "$(dirname "${BASH_SOURCE[0]}")/url_encode.sh"

encoded_password=""$(urlencode "${CS_DATABASE__PASSWORD}")



# Connect to the proxy
docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:${encoded_password}@proxy:6432/cipherstash <<-EOF
SELECT 1;
EOF

response=$(curl -s http://localhost:9930)

if [[ $response != *"cipherstash_proxy_statements_total 1"* ]]; then
    echo "error: did not see string in output: \"cipherstash_proxy_statements_total 1\""
    exit 1
fi

if [[ $response != *"cipherstash_proxy_rows_passthrough_total 1"* ]]; then
    echo "error: did not see string in output: \"cipherstash_proxy_rows_passthrough_total 1\""
    exit 1
fi

if [[ $response != *"cipherstash_proxy_rows_total 1"* ]]; then
    echo "error: did not see string in output: \"cipherstash_proxy_rows_total 1\""
    exit 1
fi

if [[ $response != *"cipherstash_proxy_statements_execution_duration_seconds{quantile=\"1\"} 0."* ]]; then
    echo "error: did not see string in output: \"cipherstash_proxy_statements_execution_duration_seconds{quantile=\"1\"} 0.\""
    exit 1
fi

if [[ $response != *"cipherstash_proxy_statements_session_duration_seconds{quantile=\"1\"} 0."* ]]; then
    echo "error: did not see string in output: \"cipherstash_proxy_statements_session_duration_seconds{quantile=\"1\"} 0.\""
    exit 1
fi

id=$(( RANDOM % 100 + 1 ))

docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:${encoded_password}@proxy:6432/cipherstash <<-EOF
INSERT INTO encrypted (id, encrypted_text) VALUES (${id}, 'hello@cipherstash.com')
EOF

response=$(curl -s http://localhost:9930)

if [[ $response != *"cipherstash_proxy_statements_total 2"* ]]; then
    echo "error: did not see string in output: \"cipherstash_proxy_statements_total 2\""
    exit 1
fi

if [[ $response != *"cipherstash_proxy_statements_passthrough_total 1"* ]]; then
    echo "error: did not see string in output: \"cipherstash_proxy_statements_passthrough_total 1\""
    exit 1
fi

if [[ $response != *"cipherstash_proxy_encrypted_values_total 1"* ]]; then
    echo "error: did not see string in output: \"cipherstash_proxy_encrypted_values_total 1\""
    exit 1
fi


docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:${encoded_password}@proxy:6432/cipherstash <<-EOF
SELECT id, encrypted_text FROM encrypted;
EOF

response=$(curl -s http://localhost:9930)

if [[ $response != *"cipherstash_proxy_statements_total 3"* ]]; then
    echo "error: did not see string in output: \"cipherstash_proxy_statements_total 3\""
    exit 1
fi

if [[ $response != *"cipherstash_proxy_statements_encrypted_total 2"* ]]; then
    echo "error: did not see string in output: \"cipherstash_proxy_statements_encrypted_total 2\""
    exit 1
fi

if [[ $response != *"cipherstash_proxy_rows_encrypted_total 1"* ]]; then
    echo "error: did not see string in output: \"cipherstash_proxy_rows_encrypted_total 1\""
    exit 1
fi


set -e

echo "----------------------------------"
echo "Prometheus exporter tests complete"
echo "----------------------------------"
