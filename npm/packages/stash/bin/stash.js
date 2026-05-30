#!/usr/bin/env node
"use strict";

// `stash` CLI launcher. Dispatches the `proxy` subcommand to the prebuilt
// cipherstash-proxy binary shipped via the per-platform npm package.
//
//   npx stash proxy [proxy args...]            # run the proxy in the foreground
//   npx stash proxy --psql [proxy args...]     # run the proxy AND open psql through it
//
// The JS layer is intentionally thin: it resolves the right native binary and
// execs it, forwarding argv, stdio, exit code and termination signals. With
// --psql it additionally waits for the proxy to report its listen address, then
// launches psql connected to the proxy, and shuts the proxy down on exit.

const { spawn, spawnSync } = require("child_process");
const os = require("os");
const { resolveProxyBinary } = require("../lib/resolve");

function usage() {
  process.stderr.write(
    "Usage: stash <command> [args...]\n\n" +
      "Commands:\n" +
      "  proxy [args...]          Run CipherStash Proxy (forwards all args)\n" +
      "  proxy --psql [args...]   Run the proxy and open a psql session through it\n"
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

if (rest.includes("--psql")) {
  runProxyThenPsql(binary, rest.filter((a) => a !== "--psql"));
} else {
  runProxy(binary, rest);
}

// --- plain proxy: exec the binary and forward everything ---------------------

function runProxy(binary, args) {
  const child = spawn(binary, args, { stdio: "inherit" });
  forwardSignals(child);
  child.on("error", (err) => {
    process.stderr.write(`stash: failed to launch proxy: ${err.message}\n`);
    process.exit(1);
  });
  child.on("exit", (code, signal) => exitFrom(code, signal));
}

// --- proxy + psql ------------------------------------------------------------

function runProxyThenPsql(binary, args) {
  if (!commandExists("psql")) {
    process.stderr.write(
      "stash: --psql requires the `psql` client on your PATH, which was not found.\n" +
        "       Install the PostgreSQL client, or run without --psql and connect manually.\n"
    );
    process.exit(1);
  }

  const conn = connectionInfo(args);

  // stdin: ignore (the proxy is a server and never reads it; psql needs the tty).
  // stdout: piped so we can detect the listen address.
  // stderr: inherited so the proxy's few status lines are visible.
  const proxy = spawn(binary, args, { stdio: ["ignore", "pipe", "inherit"] });

  let psql = null;
  let launchedPsql = false;
  let buffered = "";

  proxy.stdout.setEncoding("utf8");
  proxy.stdout.on("data", (chunk) => {
    // Echo proxy stdout to our stderr to keep our stdout clean for psql.
    process.stderr.write(chunk);
    if (launchedPsql) return;

    buffered += chunk;
    // The proxy prints e.g. "CipherStash Proxy listening on 0.0.0.0:64335".
    const match = buffered.match(/listening on \S+?:(\d+)/i);
    if (match) {
      launchedPsql = true;
      psql = launchPsql(parseInt(match[1], 10), conn, proxy);
    }
  });

  forwardSignals(proxy);

  proxy.on("error", (err) => {
    process.stderr.write(`stash: failed to launch proxy: ${err.message}\n`);
    process.exit(1);
  });

  proxy.on("exit", (code, signal) => {
    if (!launchedPsql) {
      // Proxy died before it was ready (e.g. database unreachable).
      process.stderr.write("stash: proxy exited before it was ready; not starting psql.\n");
      exitFrom(code, signal);
    }
    // If psql is running, its own exit handler drives our exit.
  });
}

function launchPsql(port, conn, proxy) {
  process.stderr.write(`stash: connecting psql to the proxy on 127.0.0.1:${port}\n`);

  // Use PG* env so we don't have to URL-escape the connection string. The proxy
  // presents as the target database, so psql uses the target's user/db; sslmode
  // is disabled because the local proxy listener is plaintext by default.
  const env = { ...process.env, PGHOST: "127.0.0.1", PGPORT: String(port), PGSSLMODE: "disable" };
  if (conn.user) env.PGUSER = conn.user;
  if (conn.dbname) env.PGDATABASE = conn.dbname;
  if (conn.password != null) env.PGPASSWORD = conn.password;

  const psql = spawn("psql", [], { stdio: "inherit", env });

  psql.on("error", (err) => {
    process.stderr.write(`stash: failed to launch psql: ${err.message}\n`);
    stop(proxy);
    process.exit(1);
  });

  psql.on("exit", (code, signal) => {
    // psql is the foreground session; when it ends, tear the proxy down.
    stop(proxy);
    exitFrom(code, signal);
  });

  return psql;
}

// Extract user / password / dbname for the psql connection from --database-url
// (preferred), individual --db-* flags, then CS_DATABASE__* env.
function connectionInfo(args) {
  const flag = (name) => {
    const i = args.indexOf(name);
    return i !== -1 ? args[i + 1] : undefined;
  };

  let fromUrl = {};
  const url = flag("--database-url");
  if (url) {
    try {
      const u = new URL(url);
      fromUrl = {
        user: decodeURIComponent(u.username) || undefined,
        password: u.password ? decodeURIComponent(u.password) : undefined,
        dbname: u.pathname.replace(/^\//, "") || undefined,
      };
    } catch {
      process.stderr.write(`stash: could not parse --database-url for psql\n`);
    }
  }

  return {
    user: flag("--db-user") ?? fromUrl.user ?? process.env.CS_DATABASE__USERNAME,
    password: flag("--db-password") ?? fromUrl.password ?? process.env.CS_DATABASE__PASSWORD,
    dbname: fromUrl.dbname ?? process.env.CS_DATABASE__NAME,
  };
}

// --- helpers -----------------------------------------------------------------

function commandExists(cmd) {
  const probe = spawnSync(cmd, ["--version"], { stdio: "ignore" });
  return !probe.error;
}

function stop(child) {
  if (child && !child.killed) child.kill("SIGTERM");
}

function forwardSignals(child) {
  for (const signal of ["SIGINT", "SIGTERM", "SIGHUP"]) {
    process.on(signal, () => {
      if (!child.killed) child.kill(signal);
    });
  }
}

function exitFrom(code, signal) {
  if (signal) {
    process.exit(128 + (os.constants.signals[signal] || 0));
  }
  process.exit(code ?? 0);
}
