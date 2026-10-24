# Quint specifications

Design-level models of CipherStash Proxy behaviour, written in [Quint](https://quint.sh) and checked with its simulator and with TLC.
They are a pilot: the CI job is not a required check, and no Rust code is tested against them yet.

| Model | What it checks |
|---|---|
| [`routing/`](routing/requirements.md) | Statement routing: can a client's plaintext reach an encrypted column? |

Start with a model's `requirements.md`, which lists its sources, scope, assumptions, properties, findings and open decisions, then read its `verification.md` for the exact results.

## Running the checks

Needs Node 22 and Java 21.
The Quint CLI is pinned in `package-lock.json`; install it here rather than globally.

```bash
cd specs/quint
npm ci --ignore-scripts
./check.sh            # typecheck, scenario tests, simulation (about 20 s)
./check.sh tlc        # exhaustive TLC checks, including expected findings (about 7 min)
./check.sh mutations  # injected faults must each be caught (about 30 s)
```

The first `quint verify` downloads the Apalache distribution, which bundles TLC, into `~/.quint`.
`check.sh` uses the simulator's TypeScript backend, because the default Rust backend downloads a binary through the GitHub API at first use.

## What each stage means

- `quint typecheck`: the model is well formed. Nothing more.
- `quint test`: the named scenarios behave as written.
- `quint run`: no violation in the sampled traces. Not a proof.
- `quint verify --backend=tlc`: the property holds in every reachable state of that finite instance. Not a proof for larger instances.
- Mutations: each injected fault produces a counterexample, so the properties are not vacuous.

## Changing a model

Decide first whether a change is a requirements change or an implementation change.
A requirements change updates `requirements.md` and the model together, and needs review of the property it changes.
Never weaken a property to make a check pass.
Nothing connects these checks to Proxy's Rust code yet, so changing Proxy alone never changes their results.
When Proxy's behaviour changes, update the model by hand to match.
If that model change means a recorded finding no longer reproduces, `check.sh tlc` fails until the finding is moved out of the expected violations.

## Tooling notes

- `npm audit` reports two high-severity advisories in `adm-zip`, which Quint uses to unpack the Apalache download. They concern extracting untrusted archives; Quint only extracts its own release download.
- Quint's LLM Kit skills (`quint-co/quint-llm-kit`) were not installed for this pilot.
