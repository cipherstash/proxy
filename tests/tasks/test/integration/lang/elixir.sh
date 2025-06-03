#!/usr/bin/env bash
#MISE description="Run Elixir integration tests for Proxy"

#!/bin/bash

set -e

echo "-------------------------------------"
echo "Starting Elixir tests"
echo "-------------------------------------"

(
cd tests/integration/elixir_test
mix deps.get
mix test
)

