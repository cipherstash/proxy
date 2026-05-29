#!/usr/bin/env bash
#
# Populate the per-platform packages with prebuilt proxy binaries.
#
# Prototype scope: builds/copies the binary for the CURRENT host into its
# platform package. In production this is replaced by a CI matrix that builds
# every target on the appropriate runner (macOS runners ad-hoc-sign for free;
# Linux runners produce glibc/musl builds), then publishes each platform package.
#
set -euo pipefail
HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "${HERE}/.." && pwd)"

case "$(uname -s)-$(uname -m)" in
  Darwin-arm64) pkg=proxy-darwin-arm64 ;;
  Darwin-x86_64) pkg=proxy-darwin-x64 ;;
  Linux-aarch64) pkg=proxy-linux-arm64 ;;
  Linux-x86_64) pkg=proxy-linux-x64 ;;
  *) echo "Unsupported host: $(uname -s)-$(uname -m)" >&2; exit 1 ;;
esac

echo "Building cipherstash-proxy (release) for host -> ${pkg}"
( cd "${REPO_ROOT}" && cargo build --release -p cipherstash-proxy )

dest="${HERE}/packages/${pkg}/bin"
mkdir -p "${dest}"
cp -f "${REPO_ROOT}/target/release/cipherstash-proxy" "${dest}/cipherstash-proxy"
chmod +x "${dest}/cipherstash-proxy"

# macOS arm64 requires at least an ad-hoc signature to execute. Binaries linked
# on macOS are ad-hoc-signed automatically, but re-assert it to be safe.
if [[ "$(uname -s)" == "Darwin" ]]; then
  codesign --force --sign - "${dest}/cipherstash-proxy" 2>/dev/null || true
fi

echo "Installed $(du -h "${dest}/cipherstash-proxy" | cut -f1) binary into ${pkg}/bin/"
