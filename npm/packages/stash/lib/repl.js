"use strict";

// A small built-in SQL shell used when `psql` isn't available on PATH. It is a
// convenience fallback, NOT a psql replacement: it runs SQL and prints results,
// plus a handful of \-commands. Pure JS (the `pg` driver) so it needs no native
// binaries.

const readline = require("readline");

// Minimal \-command help.
const HELP = `Commands:
  \\q             quit
  \\?             this help
  \\l             list databases
  \\dt            list tables
  \\d [name]      describe a table (or list tables)
  <sql>;         run SQL (statements end with ;)
`;

// Map a \-command to the SQL it runs (or a control action).
function metaCommand(line, dbname) {
  const [cmd, arg] = line.trim().split(/\s+/, 2);
  switch (cmd) {
    case "\\q":
      return { quit: true };
    case "\\?":
      return { help: true };
    case "\\l":
      return {
        sql: "SELECT datname AS name FROM pg_database WHERE datistemplate = false ORDER BY 1;",
      };
    case "\\dt":
      return {
        sql: `SELECT schemaname AS schema, tablename AS name FROM pg_catalog.pg_tables
              WHERE schemaname NOT IN ('pg_catalog','information_schema') ORDER BY 1,2;`,
      };
    case "\\d":
      if (!arg) {
        return metaCommand("\\dt", dbname);
      }
      return {
        sql: `SELECT column_name AS column, data_type AS type, is_nullable AS nullable
              FROM information_schema.columns WHERE table_name = $1 ORDER BY ordinal_position;`,
        params: [arg],
      };
    default:
      return { error: `unknown command: ${cmd} (try \\?)` };
  }
}

function cell(value) {
  if (value === null || value === undefined) return "";
  if (typeof value === "object") return JSON.stringify(value);
  return String(value);
}

// Render rows (array of objects) as an aligned table.
function formatTable(fields, rows) {
  const cols = fields.map((f) => f.name);
  const widths = cols.map((c) => c.length);
  const text = rows.map((row) =>
    cols.map((c, i) => {
      const s = cell(row[c]);
      if (s.length > widths[i]) widths[i] = s.length;
      return s;
    })
  );
  const pad = (s, w) => s + " ".repeat(w - s.length);
  const lines = [];
  lines.push(cols.map((c, i) => pad(c, widths[i])).join(" | "));
  lines.push(widths.map((w) => "-".repeat(w)).join("-+-"));
  for (const r of text) lines.push(r.map((s, i) => pad(s, widths[i])).join(" | "));
  lines.push(`(${rows.length} row${rows.length === 1 ? "" : "s"})`);
  return lines.join("\n");
}

async function runQuery(client, sql, params) {
  try {
    const res = await client.query(sql, params);
    const results = Array.isArray(res) ? res : [res];
    for (const r of results) {
      if (r.fields && r.fields.length > 0) {
        process.stdout.write(formatTable(r.fields, r.rows) + "\n");
      } else {
        process.stdout.write(`${r.command}${r.rowCount != null ? " " + r.rowCount : ""}\n`);
      }
    }
  } catch (err) {
    process.stderr.write(`ERROR: ${err.message}\n`);
  }
}

// Connect to the (proxy) endpoint and run an interactive shell until EOF / \q.
async function runRepl({ host, port, user, password, database }) {
  // Lazy require so the `pg` dependency is only loaded on this path.
  const { Client } = require("pg");
  const client = new Client({ host, port, user, password, database, ssl: false });
  await client.connect();

  const label = database || "stash";
  process.stdout.write(
    `Built-in SQL shell (psql not found). Connected to ${label} via the proxy. Type \\? for help, \\q to quit.\n`
  );

  const rl = readline.createInterface({
    input: process.stdin,
    terminal: Boolean(process.stdin.isTTY),
  });

  let buffer = "";
  const promptText = () => (buffer ? "... " : `${label}=> `);

  // Ctrl-C abandons the statement in progress (like psql); Ctrl-D (EOF) ends
  // the async iterator and exits.
  rl.on("SIGINT", () => {
    buffer = "";
    process.stdout.write("\n" + promptText());
  });

  process.stdout.write(promptText());

  try {
    // Async iteration processes one line at a time and awaits each query before
    // pulling the next, so statements (and the final one) complete in order.
    for await (const line of rl) {
      const trimmed = line.trim();

      if (buffer === "" && trimmed.startsWith("\\")) {
        const action = metaCommand(trimmed, database);
        if (action.quit) break;
        if (action.help) process.stdout.write(HELP);
        else if (action.error) process.stderr.write(action.error + "\n");
        else if (action.sql) await runQuery(client, action.sql, action.params);
      } else {
        buffer += (buffer ? "\n" : "") + line;
        if (buffer.trimEnd().endsWith(";")) {
          const sql = buffer;
          buffer = "";
          await runQuery(client, sql);
        }
      }

      process.stdout.write(promptText());
    }
  } finally {
    rl.close();
    // Don't block exit waiting for the proxy to close the socket; the caller
    // tears the proxy down and exits immediately after we return.
    client.end().catch(() => {});
  }
}

module.exports = { runRepl };
