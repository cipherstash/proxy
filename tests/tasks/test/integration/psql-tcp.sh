#!/usr/bin/env bash
#MISE description="PSQL with TCP"

#!/bin/bash

set -e
set -x

source "$(dirname "${BASH_SOURCE[0]}")/url_encode.sh"

encoded_password=$(urlencode "${CS_DATABASE__PASSWORD}")
echo "Encoded password: ${encoded_password}"

# sanity check direct connections
docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://${CS_DATABASE__USERNAME}:${encoded_password}@${CS_DATABASE__HOST}:${CS_DATABASE__PORT}/cipherstash <<-EOF
SELECT 1;
EOF

# Connect to the proxy
docker exec -i postgres psql postgresql://cipherstash:${encoded_password}@proxy:6432/cipherstash <<-EOF
SELECT 1;
EOF

# Connect to the proxy
docker exec -i postgres psql postgresql://cipherstash:${encoded_password}@proxy:6432/cipherstash <<-EOF
SELECT 1;
EOF

# Attempt with TLS
set +e
docker exec -i postgres psql postgresql://cipherstash:${encoded_password}@proxy:6432/cipherstash?sslmode=require <<-EOF
SELECT 1;
EOF
if [ $? -eq 0 ]; then
    echo "PSQL should not be able to connect via TLS"
    exit 1
fi

# Attempt with an invalid password
docker exec -i postgres psql postgresql://cipherstash:not-the-p%40ssword@proxy:6432/cipherstash <<-EOF
SELECT 1;
EOF

if [ $? -eq 0 ]; then
    echo "PSQL connected with an invalid password"
    exit 1
fi

set -e

echo "----------------------------------"
echo "PSQL TCP connection tests complete"
echo "----------------------------------"
