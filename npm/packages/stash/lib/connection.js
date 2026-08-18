"use strict";

const VALUE_OPTIONS = new Set([
  "--database-url",
  "--db-host",
  "-H",
  "--db-port",
  "-P",
  "--db-user",
  "-u",
  "--db-password",
  "-W",
  "--config-file-path",
  "-p",
  "--log-level",
  "-l",
  "--log-format",
  "-f",
]);

function optionValue(args, long, short) {
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i];
    if (arg === long || arg === short) return args[i + 1];
    if (arg.startsWith(`${long}=`)) return arg.slice(long.length + 1);
    if (short && arg.startsWith(short) && arg.length > short.length) {
      return arg.slice(short.length);
    }
  }
  return undefined;
}

function positionalDatabase(args) {
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i];
    if (arg === "--") return args[i + 1];
    if (VALUE_OPTIONS.has(arg)) {
      i += 1;
      continue;
    }
    if (arg.startsWith("-")) continue;
    return arg;
  }
  return undefined;
}

// Resolve the SQL client's connection identity using the same precedence as
// the proxy: individual CLI values, then --database-url, then environment.
function connectionInfo(args, env = process.env) {
  let fromUrl = {};
  const url = optionValue(args, "--database-url");
  if (url) {
    try {
      const parsed = new URL(url);
      fromUrl = {
        user: decodeURIComponent(parsed.username) || undefined,
        password: parsed.password ? decodeURIComponent(parsed.password) : undefined,
        dbname: decodeURIComponent(parsed.pathname.replace(/^\//, "")) || undefined,
      };
    } catch {
      // The native proxy will report the authoritative URL parse error. The
      // launcher only needs a best-effort view for the SQL client.
    }
  }

  return {
    user: optionValue(args, "--db-user", "-u") ?? fromUrl.user ?? env.CS_DATABASE__USERNAME,
    password:
      optionValue(args, "--db-password", "-W") ??
      fromUrl.password ??
      env.CS_DATABASE__PASSWORD,
    dbname: positionalDatabase(args) ?? fromUrl.dbname ?? env.CS_DATABASE__NAME,
  };
}

module.exports = { connectionInfo };
