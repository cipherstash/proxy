# Statement routing — verification record

Recorded on 2026-10-25 with Quint 0.33.0, TLC from the Apalache 0.62.1 distribution that Quint downloads, and OpenJDK 21, on a 4-core Linux container.
Run everything with `./check.sh all` from `specs/quint`; the TLC and mutation stages took 10 minutes 25 seconds.

## Instances

Every instance has two columns (`a`, `b`), one prepared statement name, and starts from every combination of column types, unresolvable columns and `mapping_errors_enabled`.
The client may use `SET CIPHERSTASH.UNSAFE_DISABLE_MAPPING` in every instance.

| Instance | Concurrent DDL | PostgreSQL domain check (A4) | Unchecked writes (A3 off) |
|---|---|---|---|
| `asBuilt` | no | off | no |
| `concurrentDdl` | yes | on | no |
| `concurrentDdlNoDomainCheck` | yes | off | no |
| `uncheckedWrites` | no | off | yes |

## Exhaustive checks (TLC)

TLC explores every reachable state of the instance; there is no step bound.
A pass means the property holds for this finite instance, not for any number of columns, statement names or connections.

| Instance | Property | Result | Distinct states | Time |
|---|---|---|---|---|
| `asBuilt` | `safety` (S1, S2, S3) | holds | 47,794 | 61 s |
| `concurrentDdl` | `safety` (S1, S2, S3) | holds | 376,012 | 452 s |
| `concurrentDdl` | S4 `noPlaintextToCurrentlyEncrypted` | violated (F1) | — | — |
| `concurrentDdlNoDomainCheck` | S3 `noPlaintextAtRest` | violated (F1) | — | — |
| `uncheckedWrites` | S1 `noPlaintextToKnownEncrypted` | violated (F2) | — | — |

`check.sh tlc` fails if a property that should hold is violated, and also if an expected counterexample disappears, so a model change that fixes a finding has to update `requirements.md` in the same change.
Changes to Proxy's Rust code do not affect these results; there is no adapter between the model and the implementation.

The S3 pass in `asBuilt` is with PostgreSQL's domain check switched off: there, Proxy's routing alone keeps plaintext out of encrypted columns.
The S3 pass in `concurrentDdl` relies on the domain check; `concurrentDdlNoDomainCheck` shows it does.

## Injected faults (TLC, `asBuilt`)

Each mutation edits a copy of the model and requires TLC to find a counterexample.

| ID | Fault | Caught by |
|---|---|---|
| M1 | A write may fall back while encrypted columns are configured (`may_fall_back` ignores `writes_encrypted`). | S1 |
| M2 | A table Proxy cannot resolve type checks as if it were native (no unresolvable-table refusal, #486). | S2 |
| M3 | No simple Query batch rule, so a fallback forwards a mapped statement as written. | S1 |
| M4 | Bind ignores a change to the mapping setting since Parse (no `PreparedSettingsChanged` refusal). | S1 |

M3 is the live behaviour on `main` before #484, where a multi-statement Query's fallback forwards the whole batch.
M4 is the model as first written, which reported a finding (F3) that Proxy does not have; see `requirements.md`.

## Simulation (sampled, not a proof)

`quint run`, TypeScript backend, seed 42, 2,000 traces of up to 12 steps each, invariant `safety`, no violation found.

| Instance | Witness | Traces reaching it |
|---|---|---|
| `asBuilt` | `ciphertextStored` | 156 |
| `asBuilt` | `readFellBack` | 423 |
| `asBuilt` | `statementRefused` | 1,258 |
| `asBuilt` | `batchRuleFired` | 35 |
| `asBuilt` | `unknownBindRefused` | 1,498 |
| `asBuilt` | `settingsChangedRefused` | 59 |
| `asBuilt` | `unsafePlaintextStored` | 270 |
| `concurrentDdl` | `ciphertextStored` | 124 |
| `concurrentDdl` | `dbRejected` | 261 |

Witness counts are reported, not enforced.
The scenario tests in `routing_test.qnt` reach each of these states deterministically, and those do fail CI.

## Scenario tests

`quint test` runs 19 scenarios against the behaviour as built and 2 with PostgreSQL's domain check on, all passing.
They include one concrete trace for each finding.

## What is not shown

- That Proxy's Rust code implements this model. There is no Quint Connect adapter yet.
- Anything about more than one client connection, more than two columns, or more than one prepared statement name.
- Anything outside the scope in `requirements.md`, notably `SET` transaction scope, `DISCARD ALL`, connection-local DDL and temporary tables.
- Liveness: no temporal properties are checked.
- The Apalache backend was not used.
