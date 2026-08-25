---
name: caddis
description: Operate the caddis conscience AND work card-first — query the
  verdict ledger, run replay around warden updates, explain blocks, and
  structure every unit of work as a card (Done-When + RED-TEST, strict
  EXECUTION when dispatching to a local model). Use when any tool call was
  blocked by caddis-warden, when starting any non-trivial task (write the
  card first), when dispatching code work to a local/weak model (the
  ladder), when the user asks what an agent did, or mentions caddis,
  cards, the ledger, the ladder, or replay.
---

# caddis — the conscience and the card ladder

A warden judges tool calls and remembers every verdict in
`~/.caddis/warden-ledger.jsonl`. Work happens as CARDS. Users memorize
nothing and plan nothing — you do, at the right moments.

## 1. Card-first is the default

Before any unit of work beyond a one-liner, write the card:

```text
---
id: CARD-<slug>
class: fix|feat|docs
owner: <agent-name>
---
# <one line>
# Done-When          (mechanically checkable, falsifiable)
# RED-TEST           (the failing proof, run BEFORE the work)
# EXECUTION          (when dispatching to a local model — see §3)
```

Validate before acting: `cargo run -p caddis-card --example validate --
card.md --strict` (strict demands EXECUTION; plain cards keep the v1
contract). Work RED-first: make the proof fail for the stated reason,
make the smallest change, make it pass, land with the evidence.

If YOU notice you have made 3+ content writes without a card this
session — stop, write the card, continue. (This same nudge arrives
mechanically from the skill environment; never fight it, never bypass a
law to make work pass — a wrong law is fixed by a card, not a detour.)

## 2. The conscience commands

```sh
grep '"deny|' ~/.caddis/warden-ledger.jsonl | tail     # nightly question
grep '"from":"NAME"' ~/.caddis/warden-ledger.jsonl | tail -20
caddis-warden --replay ~/.caddis/warden-ledger.jsonl [--from NAME] [--since 24h]
```

Replay re-judges history against the current law: every NEW-DENY is a
future false positive caught free; every FREED a fixed over-fire, and
NOW-STEERS / NO-LONGER-STEERS the soft-finding noise the change adds or
removes. **Read the coverage line before believing the counts** — only
command rows carry a replayable body, so a clean result covers the
fraction it names and no more. Run it before swapping an updated warden
binary; re-run `./onboard <name>` after the swap. A blocked call: explain the reason in one sentence; if
it looks wrong, pull the ledger row — a false positive is a card.

## 3. The ladder — dispatching to a local model

The card is the PLANNING side; the ladder adapts it to the EXECUTOR you
have. Honest claim: local execution EARNS levels by measurement — never
promise local-by-default.

**Before a model's first dispatch**: run the calibration pack — copy the
pack's cards and `fixtures/` to a scratch dir and work with that
directory as cwd (anchors and allowlists are rooted at `fixtures/`);
`plan/KEY.md` stays OUT of the dispatched scratch — it is the operator's
scoring oracle, never shown to the model under calibration. Dispatch
each card one-shot, record outcomes. Profiles live in
`~/.caddis/executor-profiles/<model>.json` via `skills/caddis/ladder.py`
— capability telemetry, not memory.

**Mechanical rules (never override by judgment):**

- start L1; +1 level only after 2 consecutive first-attempt untransformed
  accepts; any blast violation, claims violation, or retired-transform
  hit → immediate −1, floor L1
- a transform with ≥3 uses and 0 conversions is RETIRED — never proposed
  again
- fallback tax (strong-lane closures) per level is the honest cost,
  recorded in the profile; a preset SWITCH to strong-first is mechanical
  hysteresis (BC4): four consecutive non-accept outcomes under the
  current preset, never a judgment call

**The loop (bounded):** dispatch the card one-shot to the local model
with a FRESH context each attempt (no contamination). Gates decide —
never claims. On reject, classify the mode and apply ONE transform, then
retry (max 3 attempts total):

| reject mode | transform (a hypothesis — outcomes recorded) |
| --- | --- |
| partial-edit | pin the exact line: anchor narrows to the broken lines |
| no-edit | one imperative step; the card becomes a single command |
| wrong-target | re-anchor to the actually-edited path |
| claims-violation | restate claims-forbidden: output the file only |
| blast-violation | demote + shrink blast to the touched file |
| tool-error | do NOT transform — the lane is broken, fall back |

**Too thick? SPLIT, don't shrink.** If a card needs blast >3 or fails at
L1 twice, split it into ordered children (each its own strict card,
blast ≤3, `# SPLIT` marker with parent/order/of). Children run in
sequence; the parent's gate is all children green. The annex may carry
context but never broadens allowlist, blast, or level.

**Dead end (never silent):** after the 3 bounded attempts, the strong
lane closes the task. If the strong lane ALSO fails — STOP: hand the
card back to the operator with the evidence. The loop never retries
upward silently.

## 4. Doctrine

- The ledger is observability, not tamper-proof evidence (PROTOCOL.md).
- Never trim the ledger; archiving is the user's call.
- Profiles are telemetry: they answer "what level can this executor
  hold", nothing else.
- Local tests/gates override every rule here, including all of §3.
