# Adapters — the nerve

One adapter, one job: marshal a tool call to the binary's wire frame, spawn
it, apply the verdict. **No policy lives here.** That is the whole
update-resistance argument — harness APIs drift fast, and the only thing
exposed to that drift is this ~160-line file. If your harness renames an
event, this is a ten-line repair, and the law in Rust never notices.

## The file

`caddis-warden.ts` — written against the de-facto standard extension API
shape: `on("tool_call")` returns a block, `on("tool_result")` amends the
result. Extension-based harnesses typically load such files from a user
extensions directory (`~/.<harness>/extensions/` or
`~/.config/<harness>/extensions/` — see your harness's docs).

Hook-based harnesses don't need this file at all: a pre-tool-call hook that
feeds the same frame to the binary and honors its verdict gets the identical
law in ~20 lines.

## Stamping the caller

The `CALLER` constant at the top of the adapter is stamped per agent so the
shared ledger's `from:` field attributes every verdict to the caller that
made the call. Several agents can share one binary and one ledger, and the
audit stays answerable: *which of my agents tried what*. An unstamped copy
behaves identically and logs under the adapter's built-in default — safe to
copy around. The onboard script stamps it for you:

```bash
./onboard my-agent-name
```

## Two behaviors worth knowing

- A **steer** is delivered on the tool RESULT, not as a block: the action was
  legitimate, it runs, and the law arrives attached to its outcome — the
  moment it applies. The map of owed laws is keyed by tool-call id and deleted
  on delivery, so a long session cannot grow it.
- **Only written content is scanned**, never the text an edit replaces — the
  warden must never punish you for removing the very thing it dislikes.
