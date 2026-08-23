# Adapters — the nerve

One adapter, one job: marshal a tool call to the binary's wire frame, spawn it,
apply the verdict. **No policy lives here.** That is the whole update-resistance
argument — harness APIs drift fast, and the only thing exposed to that drift is
this ~160-line file. If your harness renames an event, this is a ten-line
repair, and the law in Rust never notices.

## The file

`caddis-warden.ts` — written against the de-facto standard extension API shape:
`on("tool_call")` returns a block, `on("tool_result")` amends the result.
Extension-based harnesses typically load such files from a user extensions
directory (`~/.<harness>/extensions/` or `~/.config/<harness>/extensions/` — see
your harness's docs).

## Hook-based harnesses: `claude-code/`

Hook-based harnesses use `claude-code/caddis-warden-gate.py` — a PreToolUse hook
that feeds the same frame to the binary and honors its verdict (deny → block,
steer → context, allow → silence). Register it in your settings'
`hooks.PreToolUse` with matcher `"*"` (snippet in the file's docstring). The
`from:` stamp comes from an optional `~/.caddis/lanes.json` cwd-prefix map;
unmapped sessions are stamped `claude-code`.

## Stamping the caller

The `CALLER` constant at the top of the adapter is stamped per agent so the
shared ledger's `from:` field attributes every verdict to the caller that made
the call. Several agents can share one binary and one ledger, and the audit
stays answerable: *which of my agents tried what*. An unstamped copy behaves
identically and logs under the adapter's built-in default — safe to copy around.
The onboard script stamps it for you:

```bash
./onboard my-agent-name
```

## Two behaviors worth knowing

- A **steer** is delivered on the tool RESULT, not as a block: the action was
  legitimate, it runs, and the law arrives attached to its outcome — the moment
  it applies. The map of owed laws is keyed by tool-call id and deleted on
  delivery, so a long session cannot grow it. - **Only written content is
  scanned**, never the text an edit replaces — the warden must never punish you
  for removing the very thing it dislikes.

## Scanner exceptions (written, deliberate)

Static analyzers flag two shapes here that are the design, not defects:

- `claude-code/caddis-warden-gate.py: main()` always exits 0 — including on
  internal error. The hook contract routes DECISIONS through stdout JSON
  (`deny` → block, `steer` → context), never through the exit code; a dead
  nerve must not kill the harness it guards. An always-0 exit is the
  documented invariant, recorded here as the in-repo exception.
- `skills/caddis/calibration/fixtures/*.py` carry unused parameters and
  `TODO` markers by construction: each fixture IS the red state a
  calibration card must turn green — the stub parameters are the task.
