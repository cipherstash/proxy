#!/usr/bin/env node
"use strict";

// `stash` CLI launcher. For now it dispatches the `proxy` subcommand to the
// prebuilt cipherstash-proxy binary shipped via the per-platform npm package.
//
//   npx stash proxy [proxy args...]
//
// The JS layer is intentionally thin: it resolves the right native binary and
// execs it, forwarding argv, stdio, exit code and termination signals so it
// behaves like running the binary directly.

const { spawn } = require("child_process");
const { resolveProxyBinary } = require("../lib/resolve");

function usage() {
  process.stderr.write(
    "Usage: stash <command> [args...]\n\n" +
      "Commands:\n" +
      "  proxy [args...]   Run CipherStash Proxy (forwards all args)\n"
  );
}

const [subcommand, ...rest] = process.argv.slice(2);

if (!subcommand || subcommand === "help" || subcommand === "--help" || subcommand === "-h") {
  usage();
  process.exit(subcommand ? 0 : 2);
}

if (subcommand !== "proxy") {
  process.stderr.write(`stash: unknown command '${subcommand}'\n\n`);
  usage();
  process.exit(2);
}

let binary;
try {
  binary = resolveProxyBinary();
} catch (err) {
  process.stderr.write(`stash: ${err.message}\n`);
  process.exit(1);
}

const child = spawn(binary, rest, { stdio: "inherit" });

// Forward termination signals so Ctrl-C / orchestrator shutdowns reach the
// long-running proxy rather than just the Node wrapper.
for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
  process.on(signal, () => {
    if (!child.killed) child.kill(signal);
  });
}

child.on("error", (err) => {
  process.stderr.write(`stash: failed to launch proxy: ${err.message}\n`);
  process.exit(1);
});

child.on("exit", (code, signal) => {
  if (signal) {
    // Re-raise so our exit reflects the signal (conventional 128+n).
    process.exit(128 + (require("os").constants.signals[signal] || 0));
  }
  process.exit(code ?? 0);
});
