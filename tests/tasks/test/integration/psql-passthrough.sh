#!/usr/bin/env bash
#MISE description="PSQL with passtrhough"

#!/bin/bash

set -e

source "$(dirname "${BASH_SOURCE[0]}")/url_encode.sh"

encoded_password=$(urlencode "${CS_DATABASE__PASSWORD}")

echo ${encoded_password}

# sanity check direct connections
docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://${CS_DATABASE__USERNAME}:${encoded_password}@${CS_DATABASE__HOST}:${CS_DATABASE__PORT}/cipherstash <<-EOF
SELECT 1;
EOF

# Connect to the proxy
docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:${encoded_password}@proxy:6432/cipherstash <<-EOF
SELECT 1;
EOF


# Confirm that there is indeed no config
set +e
OUTPUT="$(docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:${encoded_password}@proxy:6432/cipherstash --command 'SELECT * FROM eql_v2_configuration' 2>&1)"
retval=$?
if echo ${OUTPUT} | grep -v 'relation "eql_v2_configuration" does not exist'; then
    echo "error: did not see string in output: \"relation "eql_v2_configuration" does not exist\""
    exit 1
fi

set -e

echo "----------------------------------"
echo "Unconfigurated connection tests complete"
echo "----------------------------------"
