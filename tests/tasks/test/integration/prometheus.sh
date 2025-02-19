#!/usr/bin/env bash
#MISE description="Test Prometheus metrics are exported and updated"

#!/bin/bash

set -e


# Connect to the proxy
docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:password@proxy:6432/cipherstash <<-EOF
SELECT 1;
EOF

response=$(curl -s http://localhost:9930)

if [[ $response != *"statement_total_count 1"* ]]; then
    echo "error: did not see string in output: \"statement_total_count 1\""
    exit 1
fi

if [[ $response != *"row_passthrough_count 1"* ]]; then
    echo "error: did not see string in output: \"row_passthrough_count 1\""
    exit 1
fi

if [[ $response != *"row_total_count 1"* ]]; then
    echo "error: did not see string in output: \"row_total_count 1\""
    exit 1
fi


id=$(( RANDOM % 100 + 1 ))

docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:password@proxy:6432/cipherstash <<-EOF
INSERT INTO encrypted (id, encrypted_text) VALUES (${id}, 'hello@cipherstash.com')
EOF

response=$(curl -s http://localhost:9930)

if [[ $response != *"statement_total_count 2"* ]]; then
    echo "error: did not see string in output: \"statement_total_count 2\""
    exit 1
fi

if [[ $response != *"statement_passthrough_count 1"* ]]; then
    echo "error: did not see string in output: \"statement_passthrough_count 1\""
    exit 1
fi

if [[ $response != *"encryption_count 1"* ]]; then
    echo "error: did not see string in output: \"encryption_count 1\""
    exit 1
fi


docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:password@proxy:6432/cipherstash <<-EOF
SELECT * FROM encrypted;
EOF

response=$(curl -s http://localhost:9930)

if [[ $response != *"statement_total_count 3"* ]]; then
    echo "error: did not see string in output: \"statement_total_count 3\""
    exit 1
fi

if [[ $response != *"statement_encrypted_count 2"* ]]; then
    echo "error: did not see string in output: \"statement_encrypted_count 2\""
    exit 1
fi

if [[ $response != *"row_encrypted_count 1"* ]]; then
    echo "error: did not see string in output: \"row_encrypted_count 1\""
    exit 1
fi


set -e

echo "----------------------------------"
echo "Prometheus exporter tests complete"
echo "----------------------------------"
