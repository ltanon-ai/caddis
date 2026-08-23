# caddis

![caddis — a conscience for coding agents](assets/banner.png)

**A conscience for coding agents.**

Your AI agent can run any command. Including the ones it shouldn't.

Caddis is a tiny law engine that sits between **any** coding agent and its
tools. Every tool call — every shell command, every file write — is judged
before it runs: **allowed, steered, or denied**, with the reason, every time, in
a local ledger you can audit.

The caddisfly larva builds its own protective case from materials it finds
around it. An agent that grows a conscience from what's already on the machine.

## The one command to give any agent

If your agent can run a shell command, it can onboard itself. Tell it exactly
this:

```text
git clone https://github.com/ltanon-ai/caddis.git && cd caddis && ./onboard <your-agent-name>
```

That is the whole onboarding. The script builds the engine, installs one binary,
and then **proves itself**: it attempts a force-push through the engine, expects
the denial, and reads back its own ledger row — stamped `<your-agent-name>`. An
install that cannot show you a denial is not an install. From then on, every
tool call your agent makes is judged and recorded.

## What it does

```bash
$ git push --force origin main
caddis-warden [git.push.force-to-protected]: force-push to protected branch `main`
via a force flag: `git push --force origin main`. Shared history is not yours
to rewrite — other clones already have it.
```

- **Laws, not prompts.** Force-pushes to protected branches, credentials written
  into files, suppression markers smuggled into code — denied mechanically,
  through real shell grammar: `sudo`/`env`/`nohup` wrapper descent,
  `then`/`do`/`$()`/`bash -c` command positions, `#` comments, quoting, escaped
  separators, even getopt's own abbreviation rules. Not substring matching. -
  **Deny, steer, allow — and log everything.** HARD findings block the call.
  SOFT findings let it run and deliver the law attached to the *result* —
  doctrine at the moment it applies, not a lecture at session start. Every
  verdict lands in a local append-only ledger: what was attempted, what was
  stopped, when, why, and **which agent** did it. - **One binary, zero
  dependencies.** ~1 700 lines of Rust, three first-party crates, no external
  deps. Builds in about two seconds, runs anywhere, reads one length-prefixed
  frame and answers one JSON verdict. No cloud, no telemetry, no trust required.
  - **Universal attachment.** It does not care whose brain it guards. Any
  harness built on extensions, hooks, or headless RPC can wire it in with one
  thin adapter file — the adapter holds no policy, so harness API churn never
  touches the law. Several agents can share one binary and one ledger, each
  verdict attributed to its caller. - **Fails honestly.** Binary missing? Tools
  keep flowing, loudly — a deployment problem must not brick your agent at 3am.
  Binary ran but the verdict is unreadable? **Blocked** — a judgement you cannot
  read is not an approval. - **Reads stay free.** `read`/`grep`/`glob`/`ls` are
  never taxed — a conscience that makes reading expensive gets hated for no
  safety gain. The one exception: files that must never be read (credentials,
  vaults) are denied even to readers.

## The shape of it

![one conscience, many bodies — any number of agents, one binary, one ledger](assets/diagram-architecture.png)

Any number of agents — each with a ~160-line nerve — one stateless law engine,
one shared ledger. *"Which of my agents tried what"* is one grep.

## How the ledger works

![every decision — allow, steer and deny alike — appends one attributed row to an append-only local file](assets/diagram-ledger.png)

The ledger answers the nightly question — *"what did the agent do while I
slept?"* — which a guard that records only its refusals cannot. Every decision
appends one row: sequence number, the caller it is attributed to, the tool, the
verdict with the command's first line, a timestamp. Nothing is ever edited or
deleted; the file only grows; a gap in the sequence numbers means the guard was
not running, and says so loudly.

## The verdict flow

![a tool call arrives as one frame; allow, steer or deny leaves; every verdict is ledgered](assets/diagram-verdict.png)

## Onboarding is the proof

![build, wire, prove — the install fails unless it can show you a denial](assets/diagram-onboard.png)

## Layout

| Path | What it is |
| --- | --- |
| `crates/caddis-warden` | the law engine + decision binary |
| `crates/caddis-core` | envelope → policy → idempotency → ledger kernel |
| `crates/caddis-card` | the work-unit law (Done-When + RED-TEST schema) |
| `adapters/` | the nerve: one thin adapter file, no policy |
| `PROTOCOL.md` | the wire contract and the failure doctrine |
| `LAWS.md` | what is denied, what steers, how to add a law |
| `ONBOARD.md` | the onboarding story and the self-proof |

## Adding a law

Laws live in Rust, arrive with tests, and are written red-first: a failing test
that proves the hole, then the smallest fix that closes it. See `LAWS.md`. The
engine's own history is 25 hardening iterations of exactly this loop — every
bypass was measured against real `getopt`/`bash` behavior before it was fixed,
and the test suite (170+) pins each one.

## License

Apache-2.0.
