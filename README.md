# caddis

<p><img src="assets/logo.png" width="200" alt="caddis — a conscience for coding agents"></p>

**A conscience for coding agents.**

Your AI agent can run any command. Including the ones it shouldn't.

Caddis is a tiny law engine that sits between **any** coding agent and its tools. Every tool call — every shell command, every file write — is judged before it runs: **allowed, steered, or denied**, with the reason, every time, in a local ledger you can audit.

The caddisfly larva builds its own protective case from materials it finds around it. An agent that grows a conscience from what's already on the machine.

## What it does

```bash
$ git push --force origin main
caddis-warden [git.push.force-to-protected]: force-push to protected branch `main`
via a force flag: `git push --force origin main`. Shared history is not yours
to rewrite — other clones already have it.
```

- **Laws, not prompts.** Force-pushes to protected branches, credentials written into files, suppression markers smuggled into code — denied mechanically, through real shell grammar: `sudo`/`env`/`nohup` wrapper descent, `then`/`do`/`$()`/`bash -c` command positions, `#` comments, quoting, escaped separators, even getopt's own abbreviation rules. Not substring matching.
- **Deny, steer, allow — and log everything.** HARD findings block the call. SOFT findings let it run and deliver the law attached to the *result* — doctrine at the moment it applies, not a lecture at session start. Every verdict lands in a local append-only JSONL ledger: what was attempted, what was stopped, when, why, and **which harness** did it.
- **One binary, zero dependencies.** ~1 700 lines of Rust, three first-party crates, no external deps. Builds in seconds, runs anywhere, reads one length-prefixed frame and answers one JSON verdict. No cloud, no telemetry, no trust required.
- **Fails honestly.** Binary missing? Tools keep flowing, loudly — a deployment problem must not brick your agent at 3am. Binary ran but the verdict is unreadable? **Blocked** — a judgement you cannot read is not an approval.
- **Reads stay free.** `read`/`grep`/`glob`/`ls` are never taxed — a conscience that makes reading expensive gets hated for no safety gain. The one exception: files that must never be read (credentials, vaults) are denied even to readers.
- **Attaches to anything.** omp today; any [pi](https://pi.dev)-family agent (little-coder, prime-agent) with one extension file; Claude Code with one hook. If your harness can intercept a tool call and spawn a process, it can grow a conscience.

## Quick start

```bash
git clone https://github.com/ltanon-ai/caddis.git && cd caddis
cargo build --release
./onboard            # installs the binary, wires your harness, PROVES itself
```

The `onboard` script is not done until the conscience proves it is alive: it attempts a force-push through the engine, expects the denial, and checks its own ledger row. An install that cannot show you a denial is not an install.

## The shape of it

```
omp ──────┐                                   ┌──────────────────────────┐
little-   ├─ nerve (≈160-line adapter) ──────▶│  caddis-warden binary    │
coder ────┤   spawns per tool call            │  law engine, stateless   │
prime-    ├─ length-prefixed frame in         │  deny / steer / allow    │
agent ────┤   JSON verdict out                └────────────┬─────────────┘
claude-   │                                                ▼
code ─────┘                                   ~/.caddis/warden-ledger.jsonl
                                              one conscience, many bodies
```


<p><img src="assets/diagram-architecture.png" width="720" alt="one conscience, many bodies — four harnesses, one binary, one ledger"></p>

## Layout

| Path | What it is |
|---|---|
| `crates/caddis-warden` | the law engine + decision binary |
| `crates/caddis-core` | envelope → policy → idempotency → ledger kernel |
| `crates/caddis-card` | the work-unit law (Done-When + RED-TEST schema) |
| `adapters/` | the nerve: one TypeScript adapter per harness family |
| `PROTOCOL.md` | the wire contract and the failure doctrine |
| `LAWS.md` | what is denied, what steers, how to add a law |
| `ONBOARD.md` | the onboarding story and the self-proof |

## The verdict flow

<p><img src="assets/diagram-verdict.png" width="720" alt="a tool call arrives as one frame; allow, steer or deny leaves; every verdict is ledgered"></p>

## Onboarding is the proof

<p><img src="assets/diagram-onboard.png" width="720" alt="build, wire, prove — the install fails unless it can show you a denial"></p>

## Adding a law

Laws live in Rust, arrive with tests, and are written red-first: a failing test that proves the hole, then the smallest fix that closes it. See `LAWS.md`. The engine's own history is 25 hardening iterations of exactly this loop — every bypass was measured against real `getopt`/`bash` behavior before it was fixed, and the test suite (170+) pins each one.

## License

Apache-2.0.
