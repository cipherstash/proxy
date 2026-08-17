# CipherStash Proxy burn-in

This package drives deterministic conformance checks and a timed mixed CRUD workload through a
real Proxy into PostgreSQL. The fixture schema and seed migration are copied from pg-proto's
burn-in package so results and future benchmarks use the same type-lab and commerce model.

The database and CipherStash credentials needed by Proxy must already be available in the
environment. Start the test PostgreSQL service before either command.

```bash
cargo run -p cipherstash-proxy-burn-in -- conformance
cargo run -p cipherstash-proxy-burn-in -- soak --duration-seconds 300
```

`soak` always runs `cargo build --locked --release --package cipherstash-proxy` and starts that
exact release binary. It samples the Proxy process RSS once per second and writes the full series
to `target/burn-in/soak-report.json`. Use `--max-rss-growth-mib` to turn retained growth into a
hard failure, and `--concurrency` to adjust load.

Override connection URLs with `--proxy-database-url` / `--direct-database-url` or the
`BURN_IN_PROXY_DATABASE_URL` / `BURN_IN_DIRECT_DATABASE_URL` environment variables.

The Proxy crate also exposes the same commerce workload as a Criterion target. With PostgreSQL
and Proxy already running, execute:

```bash
cargo bench -p cipherstash-proxy --bench proxy_crud
```

Every measured iteration opens a realistic short-lived connection, performs transactional CRUD
with joins and an aggregate, validates the returned values, and removes its rows.
