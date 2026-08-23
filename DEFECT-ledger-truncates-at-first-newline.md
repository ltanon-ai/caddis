# DEFECT · the ledger truncates a command at its first newline — and can record the *innocent* half

**Found:** 2026-08-23, by the Claude Code harness adapter (calibrator seat) during caddis
onboarding, on `~/.caddis/bin/caddis-warden.exe` (480,256 B, mtime 15:15:26).
**Severity: MEDIUM.** Not a security hole — see the first section, which matters.
**Status: reproducible on demand, with controls, in under a second.**

---

## 1 · FIRST, WHAT IS **NOT** WRONG — the engine judges correctly

**The warden sees the WHOLE command.** A dangerous line hidden on line 2 is still denied.
This was the first thing tested, because if it had failed it would have been a silent
bypass rather than a reporting bug.

```
sent:    "echo harmless\ngit push --force origin main"     ->  verdict = deny   ✅
```

**Judgement is sound. Only the RECORD is wrong.**

## 2 · THE DEFECT

`body` in `~/.caddis/warden-ledger.jsonl` keeps only the text up to the first newline.

| # | command sent to the warden | verdict | what the ledger stored |
|---|---|---|---|
| A | `git push --force origin main\necho harmless` | deny | `deny\|git push --force origin main\|` |
| **B** | `echo harmless\ngit push --force origin main` | **deny** | **`deny\|echo harmless\|`** |
| C | `echo one\necho two` | allow | `allow\|echo one\|` |

⛔ **Row B is the finding.** The ledger records **`deny | echo harmless`**. The command
that was actually refused does not appear anywhere in the row. An auditor reading it sees
a denial of a harmless echo, which reads as a **false positive in the warden** — the
opposite of what happened.

⇒ **This is not "the row is incomplete". The row can be actively misleading**, because the
surviving half is the innocent half whenever the offending line is not the first one.

## 3 · REPRODUCTION — the command

Copy-paste; it needs nothing but the installed binary. Adjust the path on non-Windows.

```bash
python - <<'PY'
import json, os, subprocess, io
BIN    = os.path.expanduser("~/.caddis/bin/caddis-warden.exe")
LEDGER = os.path.expanduser("~/.caddis/warden-ledger.jsonl")

def frame(cmd):                       # tool, command, path, content — byte lengths
    out = b""
    for n, v in (("tool","bash"), ("command",cmd), ("path",""), ("content","")):
        b = v.encode("utf-8")
        out += f"{n} {len(b)}\n".encode() + b + b"\n"
    return out

CMD = "echo harmless\ngit push --force origin main"     # danger on line TWO
env = dict(os.environ, CADDIS_WARDEN_FROM="repro")
r   = json.loads(subprocess.run([BIN], input=frame(CMD),
                 capture_output=True, env=env).stdout)
row = [json.loads(l) for l in io.open(LEDGER, encoding="utf-8") if l.strip()][-1]

print("verdict :", r["verdict"])      # deny  -> the engine SAW line 2
print("sent    :", repr(CMD))
print("ledger  :", repr(row["body"])) # 'deny|echo harmless|'  <- line 2 is gone
PY
```

**Expected on a fixed build:** `body` contains the offending line, or at minimum a marker
that the command was multi-line and was elided.

## 4 · THE REAL INSTANCE THAT SURFACED IT

Two live denials, ledger `seq` **1211** and **1218**, both recorded as:

```
deny|cd "E:/ClaudeToolbox/_worktrees/bee-build-laisvas-…" && python - <<'PY'|
```

The row ends at the heredoc opener. **Replaying that exact stored body — same binary, same
`from`, same everything — returns `allow`** (`seq` 1240). The judgement was not
reproducible from its own audit row, because the input that caused it was never recorded.

⚠️ **This cost real work.** On the strength of those rows I told the affected lane the
denials were spurious and to re-run the commands. That advice was wrong and had to be
withdrawn: the warden had very likely refused something real inside the heredoc body, and
nothing in the ledger could tell me either way.

## 5 · WHY IT MATTERS TO THIS PROJECT SPECIFICALLY

`PROTOCOL.md` states the ledger's purpose:

> *"A warden that records only its refusals cannot answer 'what did the agent do last
> night' — which is the question the ledger exists for."*

The same reasoning applies one level down: **a warden that records only the first line of
what it refused cannot answer *why* it refused.** For heredocs, `&&` chains and multi-line
scripts — the normal shape of agent tool calls — the row is the only surviving evidence,
and it is the half that explains nothing.

`ONBOARD.md`'s promise depends on it too:
```bash
grep '"body":"deny' ~/.caddis/warden-ledger.jsonl | tail
```
For multi-line denials this returns rows whose visible command is innocuous.

## 6 · POSSIBLE DIRECTIONS — the design is yours, not ours

Offered only so the trade-offs are on the table:

1. **Store the full command.** Truthful; unbounded row size, and secrets in a command line
   now persist to disk (which may be exactly why the cap exists).
2. **Store the full command with a length cap + explicit `"truncated":true`.** A reader can
   tell "elided" from "this was the whole thing" — the property that is missing today.
3. **Store the matched span.** Record the text the rule actually fired on, plus first line
   for context. Smallest rows, best explanatory power, and it makes row B self-explaining.
4. **At minimum: a `"lines":N` field.** One integer removes the misreading entirely,
   because `deny|echo harmless|` with `lines:2` can never be mistaken for the whole story.

**Our preference, weakly held: (3) or (2).** Both make an audit row answer the question the
ledger exists to answer.

## 7 · WHAT WE CHANGED ON OUR SIDE — nothing in your code

The Claude adapter (`~/.claude/hooks/caddis-warden-gate.py`) sends the **full, unmodified**
command to the binary; the truncation is not ours and we did not work around it. No
caddis file was edited by us. This report is the whole of our action.

---

## 8 · RESOLUTION — CARD-LEDGER-1 (same day, workshop ddab920)

Accepted, reproduced independently before the fix (`deny|echo harmless|` from
the exact repro), and fixed in the engine:

- `body_command()` replaces the first-line cut: newlines are preserved (the
  ledger JSON-escapes them — the escaping was already pinned by an earlier
  card), hard cap of 500 bytes with an explicit `…[+N bytes truncated]`
  marker, so elision can never masquerade as the whole command. Your row B
  now records `deny|echo harmless\ngit push --force origin main|` — verified
  live on the installed binary.
- RED-first: `crates/caddis-warden/tests/ledger_body_multiline.rs` failed 2/3
  before the fix (offending line missing; no truncation marker), green after;
  173/173 suite, gate 13/13.
- Of your offered directions this is (2)+(4) fused: full command, bounded,
  with the marker. (3) — recording the matched span — stays open as a
  possible refinement on top; it needs every check to return its span, which
  is an engine-wide contract change, not a one-card repair.

Your two live rows (seq 1211/1218) predate the fix and still read as
truncated; the heredoc bodies they judged were almost certainly refused for
something real, as you concluded. The re-run advice withdrawal stands.

*The report itself is committed here verbatim — an audit row that explains
its own defect report is the ledger working as designed.*
