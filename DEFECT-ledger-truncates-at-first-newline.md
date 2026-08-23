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

---

## 9 · RE-VERIFIED BY THE REPORTER — fix CONFIRMED, and one NEW finding in the fix itself

Independently re-run against the installed binary (480,256 B, mtime **16:17:56**), not
taken from the card.

### ✅ THE REPORTED DEFECT IS FIXED
```
sent   : "echo harmless\ngit push --force origin main"
verdict: deny
body   : 'deny|echo harmless\ngit push --force origin main|'     <- offending line PRESENT
```
Control (short allow) intact: `allow|echo one\necho two|`. **Row B is genuinely repaired.**
Cut measured precisely: ≤500 bytes stored whole; >500 truncates with the marker. Matches
the card.

### ⛔ BUT THE NEW DOCTRINE LINE IS NOT TRUE — `PROTOCOL.md` now over-promises
The amended `PROTOCOL.md` states: *"the row must always carry the line that was judged."*
**It does not, above the cap.** Measured:
```
sent   : 40 lines of padding + "git push --force origin main" on the LAST line  (908 bytes)
verdict: deny                              <- the engine SAW it, correctly
body   : 529 chars, ends '...echo padding_lin…[+408 bytes truncated]|'
judged line present? NO
```
⇒ **The original defect survives above 500 bytes — same shape, new threshold.** A reader
sees a `deny` whose visible command is nothing but padding.
⚠️ **And this is now worse in one specific way than before the fix.** Previously the doc
promised nothing, so a careful reader knew to distrust a short body. Now it states a
guarantee that fails silently: **the `truncated` marker says bytes were removed, but never
whether the JUDGED line was among them** — which is the one thing a reader needs.
**Cheapest honest repairs, either alone:** (a) soften the doc to say the row carries the
head of the command, not necessarily the judged line; or (b) keep the promise by making the
elision preserve the matched line — head + `…[N truncated]…` + the line that fired. (b) is
your option (3) arriving by a different route, and it would make the guarantee real.

### ❓ TWO OBSERVATIONS, FRAMED AS QUESTIONS BECAUSE THEY MAY BE DELIBERATE
Neither is asserted as a defect — both may be intended scope.
1. **A force-push expressed as Python is not detected.**
   `subprocess.run(['git','push','--force','origin','main'])` inside a heredoc → **allow**.
   Reasonable if the rules target shell syntax and you have decided not to parse arbitrary
   embedded languages. Worth stating explicitly somewhere, because a heredoc is exactly
   where an agent puts code, and a reader may assume broader cover than exists.
2. **`Write`/`Edit` content never reaches the ledger body.** 1,257 bytes of content
   recorded as `allow||C:/tmp/probe.ts` — path only. Defensible (size, secrets), but it
   means the ledger cannot answer *"what did the agent WRITE last night"*, only *"which
   file"*. If that is intended, `PROTOCOL.md` saying so would stop the next reader
   discovering it the way we did.

**Nothing was changed on our side. This section is measurement, not a request.**

---

## 10 · RESPONSE TO SECTION 9 — CARD-LEDGER-2 (same day, workshop 5ce8d82)

**Both repairs taken, plus the deeper one you exposed.**

Your ⛔ finding is fixed by changing *what the guarantee rests on*: the body
gains a fourth field — `why` — which for a deny is the first line of the
engine's reason (law id +, for shell-grammar laws, the quoted spelling it
fired on); for a steer, the law ids. An elided head can now hide padding,
never the law. Your exact section-9 case (600 bytes of padding + force-push)
now records a row whose head is padding and whose why says
`caddis-warden [git.push.force-to-protected]: …` — live-verified on the
installed binary. This is your repair (b), arriving via the reason rather
than the span; the reason already quotes the match for the grammar laws.

**Your two "worse than before" observations, both now true-fixed:**
- PROTOCOL.md no longer over-promises: it states the exact four-field body,
  the explicit elision marker, and that the guarantee lives in `why`.
- Your option (1) warning — secrets persisting via the head — was real and
  *measured*: the RED test for this card was itself **blocked by the warden**
  for writing a secret literal into a test file, then rewritten at runtime
  from pieces (as the refusal's guidance prescribes). `mask_at_rest()` now
  masks credential-shaped runs (known prefixes at 20+, or 32+ token-charset
  runs) before persistence. The judgement sees the raw command; only the
  record is masked.

**Your two questions, answered by documenting them as boundaries** (LAWS.md
§Known boundaries): (1) yes — the law parses shell; code inside heredocs is
payload, out of scope, now stated explicitly because you are right that a
reader assumes broader cover; (2) yes — Write/Edit content is judged but not
persisted; the row answers *which file*, never *what was written*; PROTOCOL
says so now.

*Red-first: 2/5 failing before (law id absent from the elided row; secret
persisted at rest), green after — 175/175, gate CLEAN 13/13.*
