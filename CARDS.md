# Cards — the work-unit law

Caddis grew from a discipline where every unit of work is a **card**: a
small document whose completion is *falsifiable* and whose proof is
*mechanical*. The `caddis-card` crate ships that schema and its
validator; this page is how the system works, why it is useful, and how
to use it.

## What a card is

A card has YAML frontmatter (`id`, `class`, `owner`) and sections. Two
sections are mandatory — the schema rejects a card without them:

- **Done-When** — the completion criterion, in a form a machine can
  check: "pytest X passes", "grep Y finds Z". Never "looks correct".
- **RED-TEST** — how you prove the work is not lying: the failing test,
  the measured before/after, the command whose exit code settles it.

Everything else (context, steps, blast radius) is free prose.

Sections may sit at heading level one **or** two: markdown's single-H1
convention and the card schema do not fight — a card that is also a
publishable document simply demotes its sections. A fenced code block is
always **content**: an embedded document inside a ``` fence (a nested
plan, a fixture) never leaks its headings into the wrapping card's
sections.

## Why it is useful

- **Claims are noise; evidence is the artifact.** A card converts "done"
  from an assertion into a command someone can run.
- **RED-first prevents self-deception.** You write the failing proof
  *before* the work, so the work cannot quietly redefine success.
- **The unit survives the session.** A card is re-runnable, reviewable,
  and auditable long after the conversation that produced it is gone.
- **The validator is mechanical.** A missing Done-When or RED-TEST is
  a schema rejection, not a reviewer's mood.

## How to use it

```sh
# 1. write the card (card.md) — see the worked examples below
# 2. validate it against the schema
cargo run -p caddis-card --example validate -- card.md
# 3. do the work RED-first: make the RED-TEST fail for the stated reason
# 4. implement the smallest change that closes it
# 5. re-run the proof: the RED-TEST now passes; Done-When holds
# 6. land the card with the evidence attached (commit message, log tail)
```

## A worked example (v1)

```text
---
id: CARD-EX-1
class: fix
owner: my-agent
---

# CARD-EX-1: timezone bug in the daily report

## Done-When

- pytest tests/test_report.py -q reports 4 passed
- the report header shows UTC for every row

## RED-TEST

- pytest tests/test_report.py::test_header_timezone fails today with
  "expected UTC, got local" — run it before touching code.
```

Validate it, watch the test fail for exactly the stated reason, fix the
smallest thing, watch it pass. If the test fails for a *different*
reason, the card was wrong — fix the card first.

## The strict EXECUTION contract

`validate --strict` adds the contract a card destined for a local
executor must carry — every field the dispatch machinery reasons about:

| Field | Meaning |
| --- | --- |
| `level` | L1–L3; absent or invalid defaults LOW (L1) |
| `blast` | paths the card may touch; 1..=3, hard error outside |
| `claims-forbidden` | output the work only; gates decide, not claims |
| `anchors` | the EXACT current bytes of each file, verbatim |
| `allowlist` | the exact editable paths; nothing else |

A strict card, in the shape the calibration packs use:

````markdown
---
id: CARD-EX-2
class: fix
owner: my-agent
---

# CARD-EX-2: clamp the input

## Done-When

- fixtures/test_all.py clamp assertions pass

## RED-TEST

- the clamp assertions fail before the edit

## EXECUTION

```yaml
level: L2
blast: 1
claims-forbidden: true
anchors:
  - path: src/clamp.py
    content: |
      def clamp(v, lo, hi):
          # TODO implement
          return v
allowlist:
  - edit src/clamp.py
```
````

Two annexes may ride on a strict card, and neither can broaden it:

- **CONTINUATION** — carries context between chained cards (`parent`,
  `carries`, `blast-cap`). The cap is stated and strict rejects a cap
  above the card's own blast: a continuation is context, never
  jurisdiction.
- **SPLIT** — names ordered children (`parent`, `order`, `of`) when a
  card is too thick for its executor. The model can split cards
  automatically; each child is a full strict card of its own, and the
  parent's gate is all children green.

A card of `class: plan` is decomposition, not execution — a different
oracle for a different job. Done-When and RED-TEST stay mandatory (the
plan oracle runs the v1 check first); a plan ADDS two sections:
`CHILDREN` (ordered: `id`, `order`, `paths`, `symbols`) and a `REVIEW`
receipt (`reviewer`, `verdict`, `checks`):

```text
cargo run -p caddis-card --example validate -- plan.md --plan
```

The split is deliberate, and enforced in both directions: a plan never
passes `--strict` (demanding execution anchors from a decomposition
document checks the wrong thing), and a work card never needs `--plan`.

## Strict cards and the ladder (for local executors)

The ladder (`skills/caddis/`) calibrates which level a given local model
has EARNED by measurement; local execution is never the default promise.
The shipped skill carries the bounded retry loop, the mechanical
promotion/demotion rules, and the calibration packs. See the README's
ladder section for the rules in full.

## Where the schema lives

`crates/caddis-card/` — a zero-dependency parser and validator split
across `lib.rs` (parsing: frontmatter, sections at H1/H2, fences),
`execution.rs` (the strict contract and verbatim anchors) and `plan.rs`
(the plan oracle). The same law governs how caddis itself is built:
every change to this repository arrives as a card whose RED-TEST ran.
