# Memory — how caddis remembers

Caddis has exactly one memory: **the verdict ledger**. It is not a vector
store, not a database, not a model context — it is an append-only file
that remembers every decision the engine ever made, who asked for it, and
why it was judged the way it was. This page is about how that memory
works and how to use it.

## What each memory row holds

Every tool call the harness routes through the adapter produces exactly
one row in `~/.caddis/warden-ledger.jsonl` — allow, steer and deny alike
(a guard that remembers only its refusals cannot answer "what did the
agent do last night"):

| Field | What it remembers |
| --- | --- |
| `seq` | the monotonic row number — memory has an order |
| `from` | WHICH agent made the call — several agents share one memory |
| `type` | what kind of call it was (`tool.bash`, `tool.write`, …) |
| `body` | verdict; the command (capped, masked); path; the law that fired |
| `ts` | when the judgement happened |

Field-level semantics, the masking rules, and what elision markers mean
live in [PROTOCOL.md](PROTOCOL.md#the-ledger).

## How memory grows

- **One decision, one row, always.** Nothing the engine writes is ever
  rewritten or deleted by the engine; the file only grows.
- **Attribution is built in.** Every adapter stamps the caller's name
  (`from:`), so one conscience serving several agents keeps one memory
  with per-agent recall — "which of my agents tried what" is one grep.
- **Gaps are loud.** A hole in `seq` means rows were not written at that
  time (absent binary, failed write) — what that does and does not prove
  is stated exactly in [PROTOCOL.md](PROTOCOL.md).

## Reading the memory

```sh
# the nightly question: everything denied today, newest last
grep '"deny|' ~/.caddis/warden-ledger.jsonl | tail

# one agent's story
grep '"from":"my-agent"' ~/.caddis/warden-ledger.jsonl | tail -20

# was the guard even running? (gaps in seq say it was not)
grep -o '"seq":[0-9]*' ~/.caddis/warden-ledger.jsonl | tail -5
```

## What caddis memory is not

- **Not tamper-proof storage.** Append-only is the engine's behavior, not
  a protection — see the exact semantics in PROTOCOL.
- **Not content memory.** Write/Edit calls are judged but their content
  is deliberately not persisted; the row remembers *which file*, never
  *what was written* (size and secrets).
- **Not a model context.** The ledger never feeds back into any model
  automatically. It is an audit memory for the operator.

## Simulation — replaying the memory

The memory is deterministic, so it can be RE-RUN. `--replay` re-judges
every recorded command against the current law and reports the diff:

```sh
caddis-warden --replay ~/.caddis/warden-ledger.jsonl
```

```text
rows: 2847  judged: 420  unchanged: 419  new-denies: 0  freed: 1  skipped: 2427
FREED   seq=383 it's
```

This is how a law change is previewed against your own history before it
ever guards a live agent: every NEW-DENY is a future false positive you
just caught for free; every FREED is a historical over-fire the new law
fixes. Honest limits: masked and elided commands are skipped (never
guessed — the secrets doctrine outranks fidelity); write/edit content was
never stored, so those rows are skipped; directory-sensitive laws are
judged from where you run replay. Read-only by construction.

## Retention

The file grows forever by design — memory you trim is memory you cannot
audit. Archiving is the operator's choice: move or copy the file any
time (the engine starts a fresh `seq` at 1 in a new file); keep the
archives, they are the history.
