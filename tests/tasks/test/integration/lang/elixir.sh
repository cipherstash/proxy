#!/usr/bin/env bash
#MISE description="Run Elixir integration tests for Proxy"

#!/bin/bash

set -e

echo "-------------------------------------"
echo "Starting Elixir tests"
echo "-------------------------------------"

mise --env tls run postgres:up --extra-args "--detach --wait"
mise --env tls run postgres:setup
mise --env tls run proxy:down proxy-tls
mise --env tls run proxy:up --extra-args "--detach --wait"

docker compose -f tests/docker-compose.yml \
  -f tests/tasks/test/integration/lang/docker-compose-elixir.yml \
  run elixir_test bash -c "mix local.hex --force && mix deps.get && mix test"

