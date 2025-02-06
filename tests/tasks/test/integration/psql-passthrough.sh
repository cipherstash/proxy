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


echo "----------------------------------"
echo "Unconfigurated connection tests complete"
echo "----------------------------------"
