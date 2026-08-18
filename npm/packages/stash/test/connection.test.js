"use strict";

const assert = require("node:assert/strict");
const test = require("node:test");
const { connectionInfo } = require("../lib/connection");

test("reads connection details from a database URL", () => {
  assert.deepEqual(
    connectionInfo(["--database-url", "postgres://alice:s3cret@db.example/orders%20archive"], {}),
    { user: "alice", password: "s3cret", dbname: "orders archive" }
  );
});

test("individual flags override the URL in all clap-supported forms", () => {
  assert.deepEqual(
    connectionInfo(
      [
        "--database-url=postgres://url-user:url-pass@db.example/url-db",
        "-ucli-user",
        "-W",
        "cli-pass",
        "cli-db",
      ],
      {}
    ),
    { user: "cli-user", password: "cli-pass", dbname: "cli-db" }
  );
});

test("falls back to the proxy environment", () => {
  assert.deepEqual(connectionInfo([], {
    CS_DATABASE__USERNAME: "env-user",
    CS_DATABASE__PASSWORD: "env-pass",
    CS_DATABASE__NAME: "env-db",
  }), { user: "env-user", password: "env-pass", dbname: "env-db" });
});
