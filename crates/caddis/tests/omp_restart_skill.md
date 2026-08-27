---
name: restart
description: OMP session rotation — drain every running agent, land the current unit, write and audit the handoff, ARM the mechanical readiness gate, then open the successor in a split herdr pane so IT closes this session. Use when the operator types /restart, says "restart yourself", "rotate the session", "perkrauk sesiją", "paleisk naują sesiją", "persijunk", or when the restart-arm hook fires at the 50% fold-point and the current unit has landed. This is the ONLY restart skill OMP sessions read; the Claude skill at ~/.claude/skills/restart/ belongs to Claude sessions.
---

# /restart — OMP session rotation with a witnessed succession

A session cannot supervise its own disappearance. This skill exists so the
predecessor never has to: it hands its state to a successor that is already
alive, and the SUCCESSOR closes the predecessor once it has demonstrably
resumed.

**The gate is `caddis rotate`** (HMAC receipt contract, CARD-0119; per-kind
drain, CARD-0120). This skill is a THIN CALLER — it does not reimplement
drain, HMAC, or receipts. The three commands, and only these three:

```bash
caddis rotate ready --lineage <id> --kind omp --model <model-id>
caddis rotate arm --lineage <id>
caddis rotate verify --lineage <id>
```

`<id>` is the named rotation line (CARD-0134). Never omit it and never
default it. `<model-id>` is chosen at rotation time — NEVER hardcoded.
The ARM receipt carries both forward (R1). CARD-0138: export
`CADDIS_LINEAGE` in the same shell as the successor spawn so the fold
nerve (CARD-0137) is armed. Percent is per-turn, never an env at start.

## LAW (operator order, repeated 10+ times 2026-08-26)

This is an OMP skill. The successor is ALWAYS `--kind omp`, NEVER
`--kind claude`. Do not touch the Claude skill at `~/.claude/skills/restart/`
— it belongs to Claude sessions. This copy lives at
`~/.omp/skills/restart/SKILL.md` and is the ONLY restart skill OMP sessions
read.

**Second LAW (operator):** Start the successor INTERACTIVELY (omp with
model, no `-p` flag), WAIT for it to load (status `idle`), THEN prompt it
via `herdr agent prompt`. A one-shot `omp -p` is NOT a successor — it dies
after one response.

## When this runs

* `restart-arm.py` fires ONCE at 50% context and ARMS this skill.
* Fire it when **the current unit has landed**.
* A restart for a non-context reason is legitimate at ANY context level.

Never restart mid-unit. A half-landed unit is the one thing a rotation
must not interrupt.

## Phase 1 — DRAIN (never skip; this is where work gets lost)

Settle your own background tasks first (background Bash tasks, Monitors,
Workflows — things caddis cannot see because the harness tracks them
in-session with no on-disk registry). The mechanical drain gate runs
inside `caddis rotate verify` (Phase 5), so the successor re-checks the
registry before closing you.

## Phase 2 — LAND

Finish what's in flight. Prove it. Don't start what can't close.

## Phase 3 — HANDOFF

Write handoff with `/handoff` or manually to `E:/ClaudeToolbox/_handoffs/`.
Audit it (Zylė if available, self-check otherwise).

## Phase 4 — ARM (the gate)

```bash
export HOME="C:/Users/ashpac"
LINEAGE=<id>
OLD=$(herdr pane current | python -c "import json,sys;print(json.load(sys.stdin)['result']['pane']['pane_id'])")

caddis rotate ready --lineage "$LINEAGE" --kind omp --model <model-id> --pane "$OLD"
caddis rotate arm --lineage "$LINEAGE"
```

`<id>` / `$LINEAGE` is the named line. `<model-id>` is the successor
model the operator or harness chose — never baked into this skill.
`ready` writes the HMAC receipt under
`$HOME/.caddis/rotation/lines/<id>/`; `arm` writes the ARM receipt from
it. If either refuses: finish what it names, then arm again. **Never
hand-write a receipt.**

## Phase 5 — ROTATE (the OMP-native part)

Pane topology is harness-native — caddis does not own it.

**Step 1: Split a pane**
```bash
NEW=$(herdr pane split --current --direction right --ratio 0.5 \
  | python -c "import json,sys;print(json.load(sys.stdin)['result']['pane']['pane_id'])")
```

**Step 2: Start OMP INTERACTIVELY** (no `-p`!)
```bash
# Pick a UNIQUE name (check `herdr agent list` first)
export CADDIS_LINEAGE="$LINEAGE"
herdr agent start <name> --kind omp --pane "$NEW" \
  -- --cwd C:/Users/ashpac/scratch
```

**Step 3: WAIT for it to load** (status becomes `idle`)
```bash
herdr agent wait <name> --until idle --timeout 180000
```

**Step 4: ONLY THEN give it the prompt**
```bash
herdr agent prompt <name> "You are the successor OMP session, running unattended.
Read <HANDOFF PATH> and resume from it. When you have confirmed the first RESUME
step is actionable, close the predecessor — but ONLY after the gate blesses it:
run `caddis rotate verify --lineage $LINEAGE` (exit 0), then `herdr pane close $OLD`.
If verify fails, do NOT close: say so and tell the predecessor what is missing." --wait --until working
```

**Step 5: STOP.** State that the successor is live and will close this pane.

## If you are the SUCCESSOR reading this

```bash
caddis rotate verify --lineage <id>
```

`verify` checks the ARM receipt's HMAC AND runs the per-kind drain
(CARD-0120). On exit 0 it also writes `$HOME/.caddis/rotation/session.receipt`
(CARD-0125/0126). Paste that file into the handoff — it is the rotation
evidence, not memory. Then close the predecessor:

```bash
herdr pane close <OLD>
```

Non-zero means the predecessor never armed, or it still has running agents.
**Do not close it.** Say what is missing and let the predecessor finish.

## Rules
1. Successor is ALWAYS `--kind omp` — never claude, never anything else
2. Successor starts INTERACTIVE (no `-p`), waits for idle, THEN gets prompted
3. Successor closes predecessor, never the other way around
4. Handoff travels as a PATH, not pasted content
5. The gate's exit code is the truth, not anyone's word
