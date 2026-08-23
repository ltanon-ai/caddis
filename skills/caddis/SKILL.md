---
name: caddis
description: Operate the caddis conscience for the user — query the verdict
  ledger, run replay before/after warden updates, explain blocks, and
  investigate incidents. Use when a tool call was blocked by caddis-warden,
  when the user asks what an agent did (audit/incident review), when
  updating or rebuilding the warden binary (replay first), or when the user
  mentions caddis, the warden, the ledger, or replay.
---

# caddis — the agent's guide to the conscience

A caddis warden judges tool calls and remembers every verdict in
`~/.caddis/warden-ledger.jsonl`. The user should never have to memorize
its commands — you know them, and you act at the right moments.

## Act proactively at these moments

1. **A tool call was blocked by caddis-warden.** Read the reason to the
   user in one sentence (it names the law). If the block looks wrong,
   do NOT bypass it — pull the row and the surrounding context from the
   ledger and show the user; a false positive is a card, not a detour.
2. **The warden binary is being updated or rebuilt.** Before the new
   binary replaces the installed one, run replay and show the diff —
   every NEW-DENY is a future false positive caught before it happens.
   After the swap, re-run `./onboard <name>` so the install re-proves.
3. **The user reports an incident** ("what did the agent do last
   night?"). Query first, replay second (gap analysis) — see commands.
4. **The user mentions caddis / warden / ledger / replay.** You are the
   interface: run the command, show the output, explain in one line.

## The commands

```sh
# everything denied today (the nightly question)
grep '"deny|' ~/.caddis/warden-ledger.jsonl | tail

# one agent's story (attributed rows)
grep '"from":"NAME"' ~/.caddis/warden-ledger.jsonl | tail -20

# was the guard even running? (gaps in seq mean it was not)
grep -o '"seq":[0-9]*' ~/.caddis/warden-ledger.jsonl | tail -5

# replay: re-judge history against the current law, show the diff
caddis-warden --replay ~/.caddis/warden-ledger.jsonl
caddis-warden --replay ~/.caddis/warden-ledger.jsonl --from NAME      # one caller
caddis-warden --replay ~/.caddis/warden-ledger.jsonl --since 24      # last day
```

## Reading replay output

- `NEW-DENY seq=N <cmd>` — the current law denies what was allowed
  then. Before an update ships: each of these is a false positive you
  just prevented. After an incident: these are recurrences now covered.
- `FREED seq=N <cmd>` — the current law allows what was denied then: a
  historical over-fire, now fixed.
- `skipped` — masked or elided rows (secrets doctrine outranks
  simulation) and non-command tools. Never guessed.

## Doctrine

- The ledger is observability, not tamper-proof evidence; a gap in seq
  means rows were not written, and after external edits it proves
  nothing (PROTOCOL.md in the caddis repo states the exact semantics).
- Never edit or trim the ledger to "clean up" — memory you trim is
  memory you cannot audit. Archiving (move whole file) is the user's
  call.
- Never disable or uninstall the warden to make work pass; if a law is
  wrong, the fix is a card with a RED-TEST (CARDS.md in the caddis repo).
