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

CI runs a bounded PostgreSQL 17 soak with `mise run test:burn-in` and uploads
`target/burn-in/soak-report.json`. Override its defaults locally with
`BURN_IN_DURATION_SECONDS` and `BURN_IN_CONCURRENCY`.

## Idle RSS growth of a released image

`rss-idle` measures how fast the resident memory of a released Proxy image grows while nothing is connected to it.
It never builds Proxy: the measured binary is the one in the image, running in a Linux container on the same glibc allocator as production.
It needs Docker, `openssl`, and the same CipherStash credentials as the soak.

```bash
mise run burn-in:rss-idle
```

That measures `cipherstash/proxy:2.2.5` for two hours and writes `target/burn-in/rss-idle-report.json`.
Override the defaults with `RSS_IDLE_IMAGE` and `RSS_IDLE_DURATION_SECONDS`, or call the subcommand directly for the remaining options:

```bash
cargo run -p cipherstash-proxy-burn-in -- rss-idle --help
```

Each reload interval is a separate run with its own PostgreSQL and Proxy containers, and the runs execute concurrently.
The default is one run at the default 60s reload cadence and one at 1s, which packs sixty times as many reloads into the same wall-clock time.
Both `CS_DATABASE__CONFIG_RELOAD_INTERVAL` and `CS_DATABASE__SCHEMA_RELOAD_INTERVAL` are set, because Proxy 2.2.5 drives both reloaders from the config reload interval.

The rig sets up the following for each run:

- PostgreSQL accepts TCP connections only over TLS (`hostssl` is the only TCP rule in `pg_hba.conf`).
- Proxy runs with `CS_DATABASE__WITH_TLS_VERIFICATION=true` and trusts a throwaway CA installed by the image's own entrypoint through `CS_DATABASE__INSTALL_AWS_RDS_CERT_BUNDLE`.
- The entrypoint installs the EQL bundled in the image and its example schema, so the encrypt configuration has encrypted columns.
- The Proxy container has a memory limit (`--memory-limit`, default `512m`).
- No client ever connects to Proxy, including for readiness checks.

The rig samples `/proc/1/smaps_rollup` in the Proxy container every 10 seconds and records total RSS and anonymous memory.
Samples in the first 120 seconds are recorded but excluded from the fit, so startup allocation does not read as growth.

For each run the report gives, for RSS and for anonymous memory:

- `slope_bytes_per_second` and `r_squared`, from a least-squares fit over the samples after warm-up.
- `bytes_per_reload`, the growth per reload interval, in which the schema and encrypt-config reloaders each fire once.
- `bytes_per_connection`, using the TLS connections PostgreSQL logged in the fit window.
- `observed_mib_per_day`, the fitted slope of that run extrapolated to a day.
- `projected_mib_per_day_at_default_intervals`, which is `bytes_per_reload` at 1,440 reloads per day.

The projection equals the observed rate only if growth scales with the number of reloads.
A low `r_squared` means the series is noise or isolated steps rather than steady growth, so read the slope with the raw `samples` beside it.

The raw series is also appended to `target/burn-in/rss-rig/<run>/samples.jsonl` as it is collected, next to the Proxy and PostgreSQL logs, so an interrupted run keeps its evidence.
Pass `--proxy-env NAME=VALUE` to add environment to Proxy, for example `MALLOC_ARENA_MAX=2`.
