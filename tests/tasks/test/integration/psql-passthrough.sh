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


# EQL v3 has no configuration table: encrypted columns are self-configuring
# domain types (e.g. `eql_v3_text_search`) and the proxy infers the encrypt
# config directly from the schema. There is no `eql_v2_configuration` table to
# probe, so the passthrough sanity checks above are sufficient here.

echo "----------------------------------"
echo "Unconfigurated connection tests complete"
echo "----------------------------------"
