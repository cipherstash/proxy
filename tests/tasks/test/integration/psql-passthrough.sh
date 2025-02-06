#!/usr/bin/env bash
#MISE description="PSQL with passtrhough"

#!/bin/bash

set -e

# sanity check direct connections
docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://${CS_DATABASE__USERNAME}:${CS_DATABASE__PASSWORD}@${CS_DATABASE__HOST}:${CS_DATABASE__PORT}/cipherstash <<-EOF
SELECT 1;
EOF

set +e
# Connect to the proxy
docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:password@proxy:6432/cipherstash <<-EOF
SELECT * FROM cs_configuration_v1;
EOF

if [ $? -eq 0 ]; then
    echo "cs_configuration_v1 table should not exist"
    exit 1
fi

set -e

echo "----------------------------------"
echo "Unconfigurated connection tests complete"
echo "----------------------------------"
