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

`rss-idle` measures how fast the resident memory of a released Proxy image grows while nothing is connected to it, or under one client workload.
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
- Unless a client workload is requested, no client ever connects to Proxy, including for readiness checks.

The rig samples `/proc/1/smaps_rollup` in the Proxy container every 10 seconds and records total RSS and anonymous memory.
Samples in the first 120 seconds are recorded but excluded from the fit, so startup allocation does not read as growth.

For each run the report gives, for RSS and for anonymous memory:

- `slope_bytes_per_second` and `r_squared`, from a least-squares fit over the samples after warm-up.
- `bytes_per_reload`, the growth per reload interval, in which the schema and encrypt-config reloaders each fire once.
- `bytes_per_connection`, using the TLS connections PostgreSQL logged in the fit window.
- `observed_mib_per_day`, the fitted slope of that run extrapolated to a day.
- `second_half_mib_per_day`, the slope fitted to the second half of the fit window only.
- `projected_mib_per_day_at_default_intervals`, which is `bytes_per_reload` at 1,440 reloads per day.

The projection equals the observed rate only if growth scales with the number of reloads.
At the default 60s cadence RSS can keep settling in steps for most of an hour, which inflates `observed_mib_per_day`.
Read `second_half_mib_per_day` beside it, or raise `--warmup-seconds`.
A low `r_squared` means the series is noise or isolated steps rather than steady growth, so read the slope with the raw `samples` beside it.

The raw series is also appended to `target/burn-in/rss-rig/<run>/samples.jsonl` as it is collected, next to the Proxy and PostgreSQL logs, so an interrupted run keeps its evidence.
Pass `--proxy-env NAME=VALUE` to add environment to Proxy, for example `MALLOC_ARENA_MAX=2`.

### Prometheus scraping

Pass `--scrape-interval-seconds` to enable Proxy's Prometheus exporter and scrape it from a separate container on the rig network.
The scraper reuses the Proxy image for its `curl`, with the entrypoint replaced, and logs one HTTP status per scrape.
The report then adds `prometheus_scrapes` to the run and `bytes_per_scrape` to each growth figure, and the run fails if no scrape succeeds after warm-up.
For the exporter enabled but never scraped, pass `--proxy-env CS_PROMETHEUS__ENABLED=true` instead.

### Client workloads

Pass `--client-workload` to drive one kind of client activity through Proxy for the whole run.
The rig process is the client, and reaches Proxy through a port Docker publishes on the host's loopback interface.

| Workload | What each loop does | Unit |
|---|---|---|
| `tcp-probe` | Opens a TCP connection and closes it, as a Kubernetes `tcpSocket` probe does. | connect |
| `startup-probe` | Sends a startup message, reads the authentication request and closes, as `pg_isready` does. | connect |
| `login-churn` | Authenticates fully, then sends `Terminate`, as an unpooled client does. | connect |
| `idle-pool` | Authenticates, then holds the connection idle until Proxy closes it at `connection_timeout` (120 seconds by default), and reconnects, as a pool that keeps idle connections does. | connect |
| `pooled-queries` | Keeps the connection open and runs an insert, two selects, an update and a delete over the example `users` table. | query |

Proxy 2.2.5 opens its PostgreSQL connection as soon as it accepts a client's TCP connection, so even `tcp-probe` costs Proxy a TLS connect to the database.
`pooled-queries` reads and writes `encrypted_email` and `encrypted_salary`, which the image's example schema configures as EQL v2 encrypted columns, and checks that each row decrypts to what was inserted.
A pooled connection that fails is replaced, so read `errors_total` before reading its slope: a run with errors is partly a `login-churn` run.

`--client-interval-millis` (default 1000) is the period of each loop, and `--client-connections` (default 1) is the number of concurrent loops, which is the pool size for `idle-pool` and `pooled-queries`.
For `idle-pool` the interval only bounds how fast a failed connect is retried.

The report adds a `client` section with the operations and errors counted over the whole run and in the fit window.
Each growth figure adds `bytes_per_client_operation` and `projected_mib_per_day_at_assumed_client_rate`.
The assumed rate is the rate the run achieved, unless `--client-operations-per-day` gives a production rate to project at.

```bash
cargo run -p cipherstash-proxy-burn-in -- rss-idle --name login-churn \
  --duration-seconds 3600 --warmup-seconds 600 --reload-interval-seconds 60 \
  --client-workload login-churn --client-operations-per-day 8640 \
  --output target/burn-in/login-churn-report.json
```

Run client workloads on a Linux host.
On macOS, Colima reaches a published port through an SSH forward, and a churn workload breaks that forward along with the Docker socket that shares it.

### Measuring a patched binary

To compare a fix against a release, build an image that differs from the released one only in the Proxy binary, and pass it with `--image`:

```dockerfile
FROM cipherstash/proxy:2.2.5
COPY cipherstash-proxy /usr/local/bin/cipherstash-proxy
```

The report records the image identifier and the version the binary prints, so the two runs cannot be confused.

### Running variants side by side

`--name` (default `idle`) names the scenario in the report, the containers, and the directory under `rss-rig/`.
Invocations with different names can run at the same time, for example a scraped and an unscraped run:

```bash
cargo run -p cipherstash-proxy-burn-in -- rss-idle --name prom-scraped \
  --duration-seconds 7200 --reload-interval-seconds 60 --scrape-interval-seconds 15 \
  --output target/burn-in/prom/scraped-report.json
```

A reload interval longer than the run is the control: Proxy loads its schema and encrypt configuration once at startup and never reloads while it is measured.

### Measuring on another host

RSS must be measured on the architecture it is reported for.
Under emulation the container process is the emulator (`qemu-x86_64` on an arm64 Docker host), so its RSS includes the emulator's own memory.
To measure `linux/amd64` from an arm64 machine, cross-compile the rig, copy the binary to an amd64 Linux host with Docker and `openssl`, and run it there with the CipherStash credentials in its environment.
The rig sets no working directory and passes only absolute paths to Docker, so the binary does not need a checkout beside it.
