# Onboarding — one command, and the proof is the install

Any agent that can (a) intercept a tool call and (b) spawn a process can grow
a conscience. Onboarding has three steps, and the third is the point.

## 1. Build and install the binary

```bash
cargo build --release
mkdir -p ~/.caddis/bin
cp target/release/caddis-warden ~/.caddis/bin/        # Windows: caddis-warden.exe
```

The installed binary — deliberately not `target/release` — is the one the
adapter spawns. A `cargo clean` must never silently disable the conscience.

## 2. Wire your harness (one adapter file)

The adapter is ~160 lines of TypeScript with **no policy** — the nerve, not
the brain. It marshals the tool call into the wire frame, spawns the binary,
applies the verdict. Nothing else.

| Harness | Where the adapter goes | Caller stamp |
|---|---|---|
| omp | `~/.omp/agent/extensions/` (list it in `~/.omp/agent/config.yml`) | `omp` (default) |
| pi / little-coder | `~/.config/little-coder/extensions/` | set `CALLER = "little-coder"` |
| prime-agent | `~/.prime/agent/extensions/` | set `CALLER = "prime-agent"` |
| Claude Code | PreToolUse hook that spawns the binary with the same frame | `from:"claude-code"` |

The `CALLER` constant at the top of the adapter is stamped per harness so the
shared ledger's `from:` field attributes every verdict to the harness that
made the call. An unstamped copy defaults to `omp` and is safe to copy around.

## 3. The self-proof

An install that cannot show you a denial is not an install. The `onboard`
script does exactly this:

1. Feed the engine a force-push frame directly — expect **deny**.
2. Feed it an innocuous `echo` — expect **allow**.
3. Read the last two ledger rows — expect one `deny` and one `allow`, fresh
   timestamps, `from:` stamped with your harness.

If any step fails, onboarding fails, loudly. A conscience that came up mute
is the exact failure this design refuses.

## What you get

From the moment onboarding passes, every tool call the agent makes — in every
harness you wired — is judged by the same law and lands in the same ledger:

```bash
# what was denied tonight, by which harness
grep '"verdict"' ~/.caddis/warden-ledger.jsonl | grep deny | tail
```

One conscience, many bodies.
