#!/usr/bin/env bash
#MISE description="PSQL with TLS"

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

# Connect to the proxy forcing TLS
docker exec -i postgres${CONTAINER_SUFFIX} psql 'postgresql://cipherstash:password@proxy:6432/cipherstash?sslmode=require' <<-EOF
SELECT 1;
EOF

# Connect without TLS
set +e
docker exec -i postgres${CONTAINER_SUFFIX} psql 'postgresql://cipherstash:password@proxy:6432/cipherstash?sslmode=disable' <<-EOF
SELECT 1;
EOF
if [ $? -ne 2 ]; then # 2 is the return value when psql fails to connect with TLS
    echo "PSQL should not be able to connect without TLS"
    exit 1
fi

# Attempt with an invalid password
docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:not-the-password@proxy:6432/cipherstash <<-EOF
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
