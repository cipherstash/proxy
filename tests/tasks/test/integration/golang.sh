#!/usr/bin/env bash
#MISE description="Run golang integration tests for Proxy"

#!/bin/bash

set -e

connection_url=postgresql://${CS_DATABASE__USERNAME}:${CS_DATABASE__PASSWORD}@proxy:6432/${CS_DATABASE__NAME}
network_id=$(docker network ls --filter name=tests_postgres --quiet)
platform="linux/$(uname -m | sed 's/x86_64/amd64/')"

env | grep -E '^(CS|PG)'

echo
echo "connection_url: $connection_url"
echo "network_id:     $network_id"
echo "platform:       $platform"

# Build the docker image
(
cd tests/integration/golang
docker build . \
  --tag cipherstash/proxy/test-integration-golang \
  --file Dockerfile \
  --platform $platform
)


echo "-------------------------------------"
echo "✅ Docker image build complete"
echo "-------------------------------------"

# Run the integration tests
docker run \
  -e DATABASE_URL=$connection_url \
  --network $network_id \
  cipherstash/proxy/test-integration-golang


echo "-------------------------------------"
echo "✅ Golang integration tests complete"
echo "-------------------------------------"
