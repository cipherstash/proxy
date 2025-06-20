#!/usr/bin/env bash
#MISE description="PSQL with TLS"

#!/bin/bash

set -e

source "$(dirname "${BASH_SOURCE[0]}")/url_encode.sh"

encoded_password=$(urlencode "${CS_DATABASE__PASSWORD}")


# sanity check direct connections
docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://${CS_DATABASE__USERNAME}:${encoded_password}@${CS_DATABASE__HOST}:${CS_DATABASE__PORT}/cipherstash <<-EOF
SELECT 1;
EOF

# Connect to the proxy-tls
docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:${encoded_password}@proxy-tls:6432/cipherstash <<-EOF
SELECT 1;
EOF

# Connect to the proxy-tls forcing TLS
docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:${encoded_password}@proxy-tls:6432/cipherstash?sslmode=require <<-EOF
SELECT 1;
EOF

# Connect without TLS
set +e
OUTPUT="$(docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:${encoded_password}@proxy-tls:6432/cipherstash?sslmode=disable --command 'SELECT 1' 2>&1)"
retval=$?
if echo ${OUTPUT} | grep -v 'Transport Layer Security (TLS) connection is required'; then
    echo "error: did not see string in output: \"Transport Layer Security (TLS) connection is required\""
    exit 1
fi
if [ $retval -ne 2 ]; then # 2 is the return value when psql fails to connect with TLS
    echo "PSQL should not be able to connect without TLS"
    exit 1
fi

# Attempt with an invalid password
docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:not-the-p%40ssword@proxy-tls:6432/cipherstash <<-EOF
SELECT 1;
EOF

if [ $? -eq 1 ]; then
    echo "PSQL connected with an invalid password"
    exit 1
fi

set -e

echo "----------------------------------"
echo "PSQL TLS connection tests complete"
echo "----------------------------------"
