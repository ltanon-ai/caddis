# The wire protocol

The contract between a harness adapter ("the nerve") and the caddis-warden
binary ("the brain"). Neither side hand-writes a parser for a format it does
not control: the adapter writes a length-prefixed frame; the binary answers
JSON.

## Request (stdin, one frame)

Fixed field order, each field one record:

```
<name> <byte-length>\n<bytes>\n
```

Fields: `tool`, `command`, `path`, `content` — all required, any may be empty.
Lengths are **byte** counts (UTF-8), never character counts: arbitrary
payloads cannot break a byte count.

Example (the frame for a bash force-push):

```
tool 4
bash
command 28
git push --force origin main
path 0

content 0

```

The adapter decides what to scan. One rule with teeth: for edits, only the
text being **written** is scanned, never the text being replaced — the warden
must never punish you for cleaning up the very thing it dislikes.

## Response (stdout, one JSON object)

```json
{"verdict":"deny","reason":"caddis-warden [git.push.force-to-protected]: ...","law":"","seq":1030}
```

| Field | Meaning |
|---|---|
| `verdict` | `allow` \| `steer` \| `deny` |
| `reason` | human-readable; present on deny and steer |
| `law` | the law text; present on steer — deliver it ON THE RESULT |
| `seq` | ledger row number; `0` means the row could not be written (loud on stderr) |

## The failure doctrine — and the two cases are NOT the same

- **Binary missing / unspawnable → ALLOW, screaming.** A deployment problem
  must not brick the operator's agent at 3am. But a silently absent conscience
  is the exact failure this engine exists against, so it is impossible to
  miss: stderr + a UI notification on every call.
- **Binary ran but the reply is unreadable → BLOCK.** It judged and we cannot
  read the judgement; trusting that is guessing. Judgement fails closed.

## The ledger

Every decision — allow, steer and deny alike — appends one row to
`~/.caddis/warden-ledger.jsonl`:

```json
{"seq":1030,"v":1,"id":"wardn…","idem_key":"…","type":"tool.bash",
 "from":"my-agent","to":"warden","body":"allow|echo hi|","ts":1787485959}
```

A warden that records only its refusals cannot answer *"what did the agent do
last night"* — which is the question the ledger exists for. The `from` field
names the calling harness: several harnesses may share one binary and one
ledger, and the audit stays attributable.

A ledger failure does not block the tool — a full disk must not halt all work
behind an audit trail. The failure is loud and the reply carries `seq: 0`, so
an unrecorded decision is detectable rather than disguised.

## Environment

| Variable | Effect |
|---|---|
| `CADDIS_WARDEN_BIN` | override the binary path (default `~/.caddis/bin/caddis-warden[.exe]`) |
| `CADDIS_WARDEN_LEDGER` | override the ledger path |
| `CADDIS_WARDEN_FROM` | stamp the caller's name in `from:` (sanitized to `[A-Za-z0-9._-]`, ≤32 bytes; empty falls back to the built-in default) |

## Why stateless

The binary is spawned per call and holds nothing between calls: no process
lifecycle to supervise, nothing to leak, and a crash costs exactly one
decision instead of the session. The ledger on disk is the only state.
