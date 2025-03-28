#!/usr/bin/env bash
##MISE description="Test for warning about missing encrypt config. Run with mapping enabled and mapping error disabled"

# This test assumes that Proxy is running with mapping enabled with mapping error disabled

set -e

# Simulate delete config by renaming
docker exec -i postgres"${CONTAINER_SUFFIX}" psql 'postgresql://cipherstash:password@proxy:6432/cipherstash' --command 'ALTER TABLE encrypted RENAME COLUMN encrypted_text TO unconfigured_text;' >/dev/null 2>&1

set +e
TIMESTAMP=$(date -u +%Y-%m-%dT%H:%M:%SZ)
docker exec -i postgres"${CONTAINER_SUFFIX}" psql 'postgresql://cipherstash:password@proxy:6432/cipherstash?sslmode=disable' --command 'SELECT unconfigured_text from encrypted' >/dev/null 2>&1
LOG_CONTENT=$(docker logs --since "${TIMESTAMP}" proxy | tr "\n" " ")
EXPECTED='Encryption configuration may have been deleted'

# Simulate delete config by renaming
docker exec -i postgres"${CONTAINER_SUFFIX}" psql 'postgresql://cipherstash:password@proxy:6432/cipherstash' --command 'ALTER TABLE encrypted RENAME COLUMN unconfigured_text TO encrypted_text;' >/dev/null 2>&1

if echo "$LOG_CONTENT" | grep -v "${EXPECTED}"; then
    echo "error: did not see string in output: \"${EXPECTED}\""
    exit 1
fi

