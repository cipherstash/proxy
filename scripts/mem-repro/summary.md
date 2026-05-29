# Passthrough memory leak — summary

Proxy leaks memory per statement when running in **passthrough with no encrypt
config** (unconfigured deployment). Slow RSS climb → OOM. Reproduced locally;
root-caused. Configured deployments are unaffected.

## Immediate fix for the customer (no upgrade required)

Set on the affected (unconfigured/passthrough) proxy:

```
CS_DEVELOPMENT__DISABLE_MAPPING=true
```

Confirmed leak-free in testing. It's the correct setting for a deployment that
does no encryption: the proxy forwards traffic untouched and never builds the
per-statement state that leaks. Works on their current **2.1.23** — no upgrade,
no dependency on the auth issue.

Other options (only if relevant): configure at least one encrypted column (also
suppresses the leak, but only if encryption is actually wanted); raising the
memory limit / periodic restarts are stopgaps, not fixes.

## Does upgrading fix it? No.

The leak is in **both 2.1.23 and 2.2.1** — and **2.2.1 is worse**. `v2.2.1` is
the latest release and is the same commit as current `main`; the faulty code is
present there. So:

- Resolving the ZeroKMS auth regression only **unblocks** an upgrade to 2.2.1.
- That upgrade does **not** fix the leak — it would make it worse.
- A code fix (below) must be written and released. Until then, use the
  `DISABLE_MAPPING` workaround.

(The auth 401 some deployments hit on 2.2.1 is unrelated to the leak: 2.2.x
derives the CTS/ZeroKMS endpoint from the region in `CS_WORKSPACE_CRN` as part of
the per-region CTS migration. A workspace that hasn't migrated needs its
CRN/endpoint updated — tracked separately.)

## Root cause

In passthrough, the backend short-circuits before per-statement cleanup:

- `postgresql/backend.rs:175` — `if self.context.is_passthrough() { write; return }`
  returns **before** `complete_execution()` + `finish_session()`
  (`backend.rs:211-212` / `227-228`), the only drains for the per-connection
  `execute` and `session_metrics` queues.
- The frontend still enqueues for **every** statement: `start_session()`
  (`frontend.rs:380` / `722`) and `set_execute*()` (`context/mod.rs:300-302`).
- Result: each passthrough statement adds queue entries that are never popped →
  unbounded growth (~1 KB/statement measured).

Why only empty config: `is_passthrough() = encrypt_config.is_empty() ||
mapping_disabled()` (`context/mod.rs:780`). With a config present, even
plaintext statements are `is_passthrough()=false` → cleanup runs → no leak.

Why `DISABLE_MAPPING` avoids it: that path makes the **frontend** early-return at
`frontend.rs:172`, *before* `start_session()`, so the queues are never populated.

## Reproduction (Linux/glibc container, 6×50k = 300k cached extended inserts)

| Version / config | baseline | after 6 rounds | post-cooldown |
| --- | --- | --- | --- |
| 2.2.1, empty config | 36 MiB | 315 MiB | **317 MiB — no release** |
| 2.1.23, empty config | 28 MiB | 84 MiB | **83 MiB — no release** |
| 2.2.1, config present | 24 MiB | 37 MiB | 37 MiB (flat) |
| 2.2.1, empty config + `DISABLE_MAPPING=true` | 31 MiB | 34 MiB | 22 MiB (flat) |

Harness: `scripts/mem-repro/` (`soak.sh`). Allocator is irrelevant (glibc ==
jemalloc); the leak is logical retention, not fragmentation.

## The fix

Landed in **PR #395** (BUG-300): the `backend.rs` passthrough branch now calls
`complete_execution()` + `finish_session()` on execute-terminating messages, so
the `execute` / `session_metrics` queues are drained in passthrough too. Includes
a regression test (`statement_lifecycle_does_not_grow_queues`) asserting the
queues stay bounded. Verified with this harness: the empty-config soak that
climbed to 317 MiB now plateaus at ~49 MiB.
