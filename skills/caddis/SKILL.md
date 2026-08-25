---
name: caddis
description: Operate the caddis conscience AND work card-first — query the
  verdict ledger, run replay around warden updates, explain blocks, and
  structure every unit of work as a card (Done-When + RED-TEST, strict
  EXECUTION when dispatching to a local model). Use when any tool call was
  blocked by caddis-warden, when starting any non-trivial task (write the
  card first), when dispatching code work to a local/weak model (the
  ladder), when the user asks what an agent did, when a bee reports done
  (attest its card), when writing a handoff or report (paste a receipt),
  when a law keeps getting in the way (check whether it is WALLPAPER), or
  when the user mentions caddis, cards, the ledger, the ladder, replay,
  receipts, attestation or proof bundles.
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

## 2. Read the ledger AT THESE MOMENTS — not when you remember to

⛔ **This section is keyed to moments, not to commands, and that is the
whole design.** caddis spent its first two versions being write-mostly:
15k rows in, almost nothing out. 0.3.0 added five readers, and five
readers nobody invokes is the same defect one storey up. So: do not read
this as a command reference. Find the moment you are in.

| the moment you are in | run this | because |
|---|---|---|
| **writing a handoff or a report** | `caddis-warden receipt --from <you> --since <hours>` | paste it in. Your prose is then checkable against a record instead of against your memory — and the receipt names any card you left `STILL OPEN` |
| **starting a non-trivial unit** | `caddis-warden card open <card.md>` | it prints whether the card BOUNDS anything. `NOT BOUNDED` means a v1 card with no allowlist: real, allowed, and it protects nothing |
| **finishing that unit** | `caddis-warden card close` | it refuses if the card file changed since you opened it |
| **a bee says "done"** | `caddis-warden attest --card <ID>` | read `OUTSIDE` first — files it wrote that its card never declared |
| **reviewing someone else's bundle** | `caddis-warden attest --verify <bundle.json>` | non-zero exit means the bundle does not match the ledger. This is the one command that makes *the builder never grades its own work* mechanical |
| **after a red gate or a surprising block** | `caddis-warden laws --since 2` | did a law fire, or did something slip past one |
| **before swapping an updated warden binary** | `caddis-warden --replay <ledger>` | every NEW-DENY is a future false positive caught free; every FREED an over-fire you fixed |
| **when a law keeps annoying you** | `caddis-warden laws` | WALLPAPER means it fires and is routed around. Bring the number, not the irritation |
| **when the law set feels thin** | `caddis-warden propose-laws` | candidates mined from allow-then-undo. **Read the FALSIFIER line before adopting one** — most candidates would deny more real work than they would catch |

### What these answers do NOT mean

- **A receipt cannot tell you what most commands WERE.** Masking and
  elision are doing their job; on a real 24h window ~90% of command text
  is withheld. The receipt says so on every run — read that line.
- **`attest` never proves a test PASSED.** The warden fires BEFORE a tool
  runs and no row carries an exit code. `RED-TEST: ATTEMPTED` is the
  strongest honest claim available, and every bundle carries its limits.
- **`laws` worked-around counts are a HEURISTIC**, and an honest
  fix-and-retry counts the same as a circumvention. A lead, never a
  verdict about an agent.
- **Cite rows by position, not by `seq`,** for anything before
  2026-08-25: the ledger's counter was not unique for 56% of its history
  (fixed in 0.3.0, but the old rows are never rewritten).

A blocked call: explain the reason in one sentence; if it looks wrong,
pull the ledger row — a false positive is a card, never a detour.

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
