# Passthrough memory reproduction harness

Tooling used to reproduce and root-cause the passthrough memory leak (BUG-300):
Proxy leaks ~1 KB per statement when running in **passthrough with an empty
encrypt config** (no encryption configured), until the pod is OOM-killed.

See [`summary.md`](summary.md) for the write-up, root cause, and fix.

## Layout

| Path | What |
| --- | --- |
| `soak.sh` | Sustained load + RSS sampling — the script that surfaces the leak |
| `run-docker.sh` | One load run + cgroup sampling against the `proxy` container |
| `run.sh` | Same, but for a proxy running as a host process |
| `sample-rss.sh` | Standalone RSS sampler (host process) |
| `schema.sql` | Plaintext `credit_data_order_v2` table (all traffic is passthrough) |
| `loadgen/` | Go load generator (parameterised INSERTs via pgx) |
| `prepleak/` | Go probe for the named-prepared-statement retention path |
| `go.mod` | Go module covering `loadgen/` and `prepleak/` |

Raw run outputs land in `results/` (git-ignored).

## Prerequisites

- Postgres + a built proxy container (`mise run postgres:up`, `mise run proxy:up`).
- Apply the plaintext schema once: `mise run postgres:psql < scripts/mem-repro/schema.sql`.
- **No local Go needed** — `run-docker.sh` / `soak.sh` run the generator in a
  `golang` container. (`run.sh` is the host-process variant and does need Go.)

## Reproducing the leak

The trigger is an **empty encrypt config**. Make the config empty, then soak:

```bash
# 1. Empty the encrypt config (unconfigured passthrough)
mise run postgres:psql -c "UPDATE eql_v2_configuration SET state='inactive' WHERE state='active';"
# 2. (re)start the proxy so it loads the empty config, then drive sustained load
scripts/mem-repro/soak.sh empty-config 6 50000 16
```

`soak.sh <label> [rounds] [count] [workers]` runs N rounds of inserts and samples
the proxy container's cgroup `memory.current` throughout (plus a cooldown to see
whether RSS is released). On an affected build RSS climbs monotonically and does
not drop; on a fixed build it plateaus.

## Variants

Compare by restarting the proxy with different env / config, then re-running the soak:

| Variant | How |
| --- | --- |
| baseline (glibc) | empty config, default settings |
| `MALLOC_ARENA_MAX=2` | set the env on the proxy — tests glibc arena bloat (ruled out) |
| `DISABLE_MAPPING` | `CS_DEVELOPMENT__DISABLE_MAPPING=true` — confirmed leak-free (the workaround) |
| config present (control) | restore an `active` row in `eql_v2_configuration` — no leak |

> An allocator comparison (jemalloc) was also run during the investigation and
> showed no difference (the leak is logical retention, not fragmentation). The
> jemalloc global-allocator feature is **not** included here — it is tracked as a
> separate decision.

## Named-statement probe

```bash
# inside the golang container, or locally from this dir:
go run ./prepleak -count 100000 -dsn '<dsn>'
```
Exercises the separate (bounded, freed-on-disconnect) named-prepared-statement
retention noted in `analysis.md`.

## Notes

- Run on Linux for production-faithful numbers (glibc arena behaviour, cgroup
  memory). macOS is directional only.
- Default proxy log level is `debug`, which is allocation-heavy; set
  `CS_LOG__LEVEL=warn` for clean memory measurements.
