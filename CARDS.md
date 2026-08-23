# Cards — the work-unit law

Caddis grew from a discipline where every unit of work is a **card**: a
small document whose completion is *falsifiable* and whose proof is
*mechanical*. The `caddis-card` crate ships that schema and its
validator; this page is how the system works, why it is useful, and how
to use it.

## What a card is

A card has frontmatter (`id`, `class`, `owner`) and sections. Two
sections are mandatory — the schema rejects a card without them:

- **Done-When** — the completion criterion, in a form a machine can
  check: "pytest X passes", "grep Y finds Z". Never "looks correct".
- **RED-TEST** — how you prove the work is not lying: the failing test,
  the measured before/after, the command whose exit code settles it.

Everything else (context, steps, blast radius) is free prose.

## Why it is useful

- **Claims are noise; evidence is the artifact.** A card converts "done"
  from an assertion into a command someone can run.
- **RED-first prevents self-deception.** You write the failing proof
  *before* the work, so the work cannot quietly redefine success.
- **The unit survives the session.** A card is re-runnable, reviewable,
  and auditable long after the conversation that produced it is gone.
- **The validator is mechanical.** A missing Done-When or RED-TEST is a
  schema rejection, not a reviewer's mood.

## How to use it

```sh
# 1. write the card (card.md) — see the worked example below
# 2. validate it against the schema
cargo run -p caddis-card --example validate -- card.md
# 3. do the work RED-first: make the RED-TEST fail for the stated reason
# 4. implement the smallest change that closes it
# 5. re-run the proof: the RED-TEST now passes; Done-When holds
# 6. land the card with the evidence attached (commit message, log tail)
```

## A worked example

```text
---
id: CARD-EX-1
class: fix
owner: my-agent
---

# CARD-EX-1: timezone bug in the daily report

# Done-When
- pytest tests/test_report.py -q reports 4 passed
- the report header shows UTC for every row

# RED-TEST
- pytest tests/test_report.py::test_header_timezone fails today with
  "expected UTC, got local" — run it before touching code.
```

Validate it, watch the test fail for exactly the stated reason, fix the
smallest thing, watch it pass. If the test fails for a *different*
reason, the card was wrong — fix the card first.

## Strict cards and the ladder (for local executors)

`validate --strict` adds the EXECUTION contract: verbatim CURRENT
anchors, a change allowlist, blast <= 3 (hard error above), level
L1-L3 (defaults LOW), claims-forbidden. A CONTINUATION annex carries
context between chained cards but may never broaden them; a SPLIT
marker names ordered children when a card is too thick for the
executor — the model can split cards automatically, each child a full
strict card. The ladder (skills/caddis) calibrates which level a given
local model has EARNED by measurement; local execution is never the
default promise. See the shipped skill for the bounded retry loop and
its mechanical promotion/demotion rules.

## Where the schema lives

`crates/caddis-card/` — a zero-dependency parser and validator (~140
lines) plus the `validate` example used above. The same law governs how
caddis itself is built: every change to this repository arrives as a
card whose RED-TEST ran.
