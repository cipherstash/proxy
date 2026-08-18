"use strict";

// Maps the current platform to the per-platform npm package that ships the
// matching prebuilt `cipherstash-proxy` binary. This mirrors the esbuild /
// Biome / SWC distribution pattern: the meta package declares each of these as
// an optionalDependency with `os`/`cpu` constraints, so npm installs only the
// one matching the host.
const PLATFORM_PACKAGES = {
  "darwin-arm64": "@cipherstash/proxy-darwin-arm64",
  "darwin-x64": "@cipherstash/proxy-darwin-x64",
  "linux-x64": "@cipherstash/proxy-linux-x64",
  "linux-arm64": "@cipherstash/proxy-linux-arm64",
  // win32-x64 would go here once we ship a Windows build.
};

function platformKey() {
  return `${process.platform}-${process.arch}`;
}

function binaryName() {
  return process.platform === "win32"
    ? "cipherstash-proxy.exe"
    : "cipherstash-proxy";
}

// Resolve the absolute path to the proxy binary for this platform, or throw a
// clear, actionable error if the platform package isn't installed.
function resolveProxyBinary() {
  const key = platformKey();
  const pkg = PLATFORM_PACKAGES[key];
  if (!pkg) {
    throw new Error(
      `cipherstash-proxy is not available for this platform (${key}). ` +
        `Supported: ${Object.keys(PLATFORM_PACKAGES).join(", ")}.`
    );
  }
  try {
    // require.resolve finds the binary inside the installed platform package.
    return require.resolve(`${pkg}/bin/${binaryName()}`);
  } catch {
    throw new Error(
      `The platform package '${pkg}' is not installed.\n` +
        `npm should install it automatically as an optionalDependency for ${key}.\n` +
        `If you used '--no-optional' or '--omit=optional', reinstall without it.`
    );
  }
}

module.exports = { resolveProxyBinary, platformKey, PLATFORM_PACKAGES };
