# Support matrix

"Supported" means: this exact adapter ran a live self-proof — a force-push
denied by the engine, its ledger row read back, the caller stamped — on a
real harness, reproducibly. "Protocol-compatible" means: the wire contract
is enough to wire it, but we have not run that integration. Nothing on
this page is aspirational.

The matrix, in full (each row links to its claim's repro below):

| Adapter | Status | Intercepts | On failure | Limits |
| --- | --- | --- | --- | --- |
| extension `.ts` | supported | call + result | allow-loud/block | routed only |
| Claude Code `.py` | supported | PreToolUse | allow-loud/block | per-machine |
| generic hook | proto-compat. | what you route | you implement | untested |
| headless / RPC | proto-compat. | what you route | you implement | untested |

- **extension `.ts`** — `adapters/caddis-warden.ts`, any harness that
  loads extension-API files: intercepts tool calls (block) and results
  (amend/steer). Absent binary allows loudly; unreadable verdict blocks.
  Sees only what the harness routes through it.
- **Claude Code `.py`** — `adapters/claude-code/caddis-warden-gate.py`,
  a PreToolUse hook: intercepts Bash, PowerShell, Write, Edit,
  NotebookEdit. Same failure doctrine; the warning rides stderr plus a
  context line. Registered per machine; a stand-aside mode exists for
  fleets wiring only some directories.
- **generic hook / RPC** — protocol-compatible: the wire contract
  (PROTOCOL.md) is enough to wire any harness that can intercept a call
  and spawn a process; we have not run those integrations.

## Contract, per supported adapter

Both supported adapters answer the same questions the same way:

- **Tool coverage.** The extension adapter scans the tool's command, path
  and written content fields; the hook adapter scans the tool_input shapes
  listed above. Unknown tools degrade to scanning what is there —
  over-scanning, never a silent hole.
- **Edits scan only what is written.** `old_string` / replaced text is
  never sent: the warden must never punish you for removing the very thing
  it dislikes.
- **Caller identity.** `CADDIS_WARDEN_FROM` (or the stamped `CALLER`
  constant) puts the harness's name in every ledger row.
- **Steer delivery.** A steer does not block: the law arrives attached to
  the tool *result* — the moment it applies.
- **Missing binary.** Loud: stderr plus a UI notification / context line
  on every call. It is a warning, not a block (see THREAT-MODEL).
- **Can the hook be bypassed?** Yes — removing the adapter or replacing
  the binary disarms everything. Trust assumptions live in
  [THREAT-MODEL.md](THREAT-MODEL.md).
- **Automatic self-test.** `./onboard <agent-name>` proves engine + ledger
  end-to-end; re-run it after any update or adapter change.

## Reproducing each "supported" claim

Extension adapter — against any harness that loads it:

```sh
./onboard my-agent     # frame-level: deny + allow + ledger row
```

Then, inside the harness, attempt a force-push: it must be blocked with a
`caddis-warden [...]` reason naming the law.

Claude Code hook:

```sh
echo '{"cwd":"/tmp","tool_name":"Bash","tool_input":
{"command":"git push --force origin main"}}' \
  | python adapters/claude-code/caddis-warden-gate.py
```

Expected: `{"decision":"block","reason":"caddis-warden [...]"}` naming
`git.push.force-to-protected`.

## Version note

Both supported adapters were proven live on 2026-08-23 against engine
build `8ed3ef0` (three extension-API harnesses and one hook-based
harness, force-push denials in the shared ledger). When the engine
changes, re-run the self-proof — an adapter claim without a fresh proof
is history, not support.
