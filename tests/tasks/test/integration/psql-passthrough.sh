#!/usr/bin/env bash
#MISE description="PSQL with passtrhough"

#!/bin/bash

set -e

# sanity check direct connections
docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://${CS_DATABASE__USERNAME}:${CS_DATABASE__PASSWORD}@${CS_DATABASE__HOST}:${CS_DATABASE__PORT}/cipherstash <<-EOF
SELECT 1;
EOF

# Connect to the proxy
docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:password@proxy:6432/cipherstash <<-EOF
SELECT 1;
EOF

# Confirm that there is indeed no config
set +e
OUTPUT="$(docker exec -i postgres${CONTAINER_SUFFIX} psql 'postgresql://cipherstash:password@proxy:6432/cipherstash?sslmode=disable' --command 'SELECT * FROM cs_configuration_v1' 2>&1)"
retval=$?
if echo ${OUTPUT} | grep -v 'relation "cs_configuration_v1" does not exist'; then
    echo "error: did not see string in output: \"relation "cs_configuration_v1" does not exist\""
    exit 1
fi

set -e

echo "----------------------------------"
echo "Unconfigurated connection tests complete"
echo "----------------------------------"
