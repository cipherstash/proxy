# CipherStash Proxy burn-in

This package drives deterministic conformance checks and a timed mixed CRUD workload through a
real Proxy into PostgreSQL. The fixture schema and seed migration are adapted from pg-proto's
burn-in package so results use the same type-lab and commerce model while exercising EQL domains.

Start the test PostgreSQL service and configure the CipherStash credentials used by Proxy in
`mise.local.toml`:

```toml
[env]
CS_WORKSPACE_CRN = "crn:region:workspace-id"
CS_CLIENT_ACCESS_KEY = "your-access-key"
CS_DEFAULT_KEYSET_ID = "your-keyset-id"
CS_CLIENT_ID = "your-client-id"
CS_CLIENT_KEY = "your-client-key"
```

The commands inherit these values from the environment when they launch Proxy.
The target database must also have EQL installed. If it does not, the burn-in installs
`cipherstash-encrypt.sql` automatically; run `mise run eql:download` first or provide a different
file with `--eql-path` / `BURN_IN_EQL_PATH`.

```bash
cargo run -p cipherstash-proxy-burn-in -- conformance
cargo run -p cipherstash-proxy-burn-in -- soak --duration-seconds 300
```

`soak` always runs `cargo build --locked --release --package cipherstash-proxy` and starts that
exact release binary. It samples the Proxy process RSS once per second and writes the full series
to `target/burn-in/soak-report.json`. Use `--max-rss-growth-mib` to turn retained growth into a
hard failure, and `--concurrency` to adjust load. The workload creates public fixture tables with
EQL domain columns and verifies CRUD through Proxy, so its memory measurements include encryption
and decryption work.

Override connection URLs with `--proxy-database-url` / `--direct-database-url` or the
`BURN_IN_PROXY_DATABASE_URL` / `BURN_IN_DIRECT_DATABASE_URL` environment variables.
