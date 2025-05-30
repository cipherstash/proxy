#!/usr/bin/env bash
#MISE description="Test Prometheus metrics are exported and updated"

#!/bin/bash

set -e

source "$(dirname "${BASH_SOURCE[0]}")/url_encode.sh"

encoded_password=$(urlencode "${CS_DATABASE__PASSWORD}")

docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:${encoded_password}@proxy:6432/cipherstash <<-EOF
TRUNCATE encrypted;
EOF

# Connect to the proxy
docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:${encoded_password}@proxy:6432/cipherstash <<-EOF
INSERT INTO encrypted (id, plaintext) VALUES (1, 'One');
INSERT INTO encrypted (id, plaintext) VALUES (2, 'Two');
INSERT INTO encrypted (id, plaintext) VALUES (3, 'Three');
INSERT INTO encrypted (id, plaintext) VALUES (4, 'Four');
INSERT INTO encrypted (id, plaintext) VALUES (5, 'Five');
EOF

docker exec -i proxy cipherstash-proxy encrypt --table encrypted --columns plaintext=encrypted_text  --verbose

docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:${encoded_password}@proxy:6432/cipherstash <<-EOF
SELECT * FROM encrypted;
EOF

set +e
OUTPUT="$(docker exec -i postgres${CONTAINER_SUFFIX} psql 'postgresql://cipherstash:${encoded_password}@proxy:6432/cipherstash?sslmode=disable' --command 'SELECT id FROM encrypted WHERE encrypted_text IS NULL' 2>&1)"
retval=$?
if echo ${OUTPUT} | grep -v '(0 rows)'; then
    echo "error: did not see string in output: \"(0 rows)\""
    exit 1
fi

set +e
OUTPUT="$(docker exec -i postgres${CONTAINER_SUFFIX} psql 'postgresql://cipherstash:${encoded_password}@proxy:6432/cipherstash?sslmode=disable' --command "SELECT id FROM encrypted WHERE encrypted_text = 'Three'" 2>&1)"
retval=$?
if echo ${OUTPUT} | grep -v '(1 row)'; then
    echo "error: did not see string in output: \"(0 rows)\""
    exit 1
fi


docker exec -i postgres${CONTAINER_SUFFIX} psql postgresql://cipherstash:${encoded_password}@proxy:6432/cipherstash <<-EOF
TRUNCATE encrypted;
EOF

set -e

echo "----------------------------------"
echo "Migrator tests complete"
echo "----------------------------------"
