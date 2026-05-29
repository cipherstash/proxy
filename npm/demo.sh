#!/usr/bin/env bash
#
# End-to-end local proof of `npx stash proxy`:
#   1. build + install the host binary into its platform package
#   2. npm install the meta package (resolves the matching platform package
#      via os/cpu-filtered optionalDependencies)
#   3. invoke through npx and through the `stash` bin, exercising arg passthrough
#
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
META="${HERE}/packages/stash"

echo "== 1. populate host platform binary =="
bash "${HERE}/build-binaries.sh"

echo "== 2. install meta package (resolves platform optionalDependency) =="
( cd "${META}" && npm install --silent )
echo "installed platform packages:"
ls "${META}/node_modules/@cipherstash" 2>/dev/null || echo "  (none — check os/cpu match)"

echo "== 3a. npx <local> proxy --version =="
( cd "${META}" && npx . proxy --version )

echo "== 3b. npx <local> proxy --help (subcommand passthrough) =="
( cd "${META}" && npx . proxy --help | head -20 )

echo "== 3c. exit codes are forwarded =="
( cd "${META}" && npx . proxy --version ) >/dev/null 2>&1; echo "  proxy --version -> exit $? (expect 0)"
( cd "${META}" && npx . frobnicate ) >/dev/null 2>&1; echo "  unknown subcommand -> exit $? (expect non-zero)"

echo "== done =="
