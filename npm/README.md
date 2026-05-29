# `npx stash proxy` — npm distribution prototype

Proof-of-concept for shipping CipherStash Proxy through npm so it runs as
`npx stash proxy [...]`, with **no native bindings** — the npm package is a thin
launcher around the existing prebuilt Rust binary (the esbuild / Biome / SWC
pattern).

## Why this shape (not N-API bindings)

Proxy is a standalone server (its own tokio runtime, listeners, TLS, signals).
We don't need to call it from JS in-process, so a native Node addon would add
lifecycle/complexity for no benefit. Instead: npm *distributes* the binary and a
tiny JS shim *launches* it.

## Layout

```
npm/
  packages/
    stash/                       # meta package — `bin: stash`
      bin/stash.js               # dispatch `stash proxy [...]` -> exec binary
      lib/resolve.js             # pick the platform package for this host
      package.json               # optionalDependencies = the platform packages
    proxy-darwin-arm64/          # one package per target, each ships one binary
    proxy-darwin-x64/            #   package.json sets os/cpu so npm installs
    proxy-linux-x64/             #   only the matching one on a given host
    proxy-linux-arm64/
  build-binaries.sh              # populate the host's platform package
  demo.sh                        # end-to-end local proof
```

How resolution works: the meta package lists each `@cipherstash/proxy-<os>-<arch>`
as an **optionalDependency**. Each platform package declares `os`/`cpu`, so npm
installs only the one matching the host. The shim `require.resolve()`s the binary
from that package and `exec`s it, forwarding argv, stdio, exit code, and signals.

## Try it (local, no registry)

```bash
bash npm/demo.sh
```

This builds the host binary, `npm install`s the meta package (which pulls in just
the matching platform package), then runs `npx . proxy --version`,
`... proxy --help`, and an unknown-subcommand case. The binaries are git-ignored
build artifacts; `build-binaries.sh` regenerates them.

> Locally we use `file:` optionalDependencies and `npx .` so it works offline.
> In production these become published, versioned packages and the command is
> literally `npx stash proxy` (or `npx @cipherstash/stash proxy`).

## What production needs

1. **Release CI matrix** builds `cipherstash-proxy` for every target:
   - macOS arm64 / x64 — **build on a macOS runner** so the linker ad-hoc-signs
     for free (enough to run on Apple Silicon; **no Developer ID / notarization
     needed** for CLI-installed binaries — npm doesn't set the Gatekeeper
     quarantine attribute).
   - Linux x64 / arm64 (glibc; add musl for Alpine if wanted).
   - Windows x64 later (`.exe`; CLI use sidesteps SmartScreen).
2. Publish each platform package (`@cipherstash/proxy-<os>-<arch>`) plus the meta
   `stash` package, all at the same version, pinned exactly.
3. The meta package's `optionalDependencies` reference the published versions
   instead of `file:` paths.

## The code-signing win (the original motivation)

- **Avoided:** macOS notarization + Developer ID certificates, and Windows
  Authenticode — the expensive, account-bound parts. npm-installed CLI binaries
  aren't quarantined, so Gatekeeper/SmartScreen don't block them.
- **Still required (but free/automatic):** an *ad-hoc* signature on Apple
  Silicon, which the macOS linker applies during the build. `build-binaries.sh`
  re-asserts it with `codesign -s -`.

## Caveats

- Requires Node/npx on the host. Great for dev laptops & CI; **k8s should keep
  using the Docker image / raw binary** — npm is an additional channel.
- `npx` for a long-running server is slightly unconventional but works (runs in
  the foreground; signals are forwarded).
- This is a throwaway prototype: packages are `private: true` and versioned
  `0.0.0-prototype` to prevent accidental publish.
