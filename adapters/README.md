# Adapters — the nerve

One adapter, one job: marshal a tool call to the binary's wire frame, spawn
it, apply the verdict. **No policy lives here.** That is the whole
update-resistance argument — harness APIs drift fast, and the only thing
exposed to that drift is this ~160-line file. If your harness renames an
event, this is a ten-line repair, and the law in Rust never notices.

## The file

`caddis-warden.ts` — written for the [pi](https://pi.dev) extension API
(`pi.on("tool_call")` / `pi.on("tool_result")`), which omp, little-coder and
prime-agent all speak.

## Per-harness wiring

| Harness | Destination | Stamp |
|---|---|---|
| omp | `~/.omp/agent/extensions/caddis-warden.ts` + list in `~/.omp/agent/config.yml` | leave `CALLER = "omp"` |
| little-coder | `~/.config/little-coder/extensions/caddis-warden.ts` | `CALLER = "little-coder"` |
| prime-agent | `~/.prime/agent/extensions/caddis-warden.ts` | `CALLER = "prime-agent"` |
| Claude Code | a PreToolUse hook spawning the binary with the same frame | `CADDIS_WARDEN_FROM=claude-code` |

Stamping the `CALLER` constant attributes every ledger row to your harness —
several harnesses can share one binary and one ledger, and the audit stays
answerable: *which of my agents tried what*. An unstamped copy behaves
identically and logs as `omp`.

## Two behaviors worth knowing

- A **steer** is delivered on the tool RESULT, not as a block: the action was
  legitimate, it runs, and the law arrives attached to its outcome — the
  moment it applies. The map of owed laws is keyed by tool-call id and deleted
  on delivery, so a long session cannot grow it.
- **Only written content is scanned**, never the text an edit replaces — the
  warden must never punish you for removing the very thing it dislikes.
