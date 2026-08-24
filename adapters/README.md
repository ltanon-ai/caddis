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
steer → additionalContext on the same PreToolUse decision, allow → silence).
Register it in your settings' `hooks.PreToolUse` with matcher `"*"` (snippet in
the file's docstring).

## Kernel-based harnesses: `rlm/`

rlm-style harnesses run the model through a persistent IPython kernel, so
every effect is a Python call and the shell-shaped ones funnel through
`subprocess.*` and `os.system`/`os.popen`. `rlm/warden_repl.py` wraps THAT
surface — the standard library, never the harness's internals, so harness
versions cannot break the nerve:

```python
import warden_repl; warden_repl.install()   # from a cell or sitecustomize
```

deny raises `WardenRefusal` with the warden's reason (the call never runs;
the exception text is the feedback the kernel shows the model); steer
writes the law beside the output and runs; allow is silent. Every warden
spawn is stamped `CADDIS_WARDEN_FROM=rlm` for ledger attribution.

⚠ **The boundary, stated honestly:** this wraps SHELL exec. A pure-Python
destructive act with no subprocess call — `shutil.rmtree(...)`,
`open(path, "w")`, `os.remove` — is OUT of scope, the same register as
THREAT-MODEL's embedded-program boundary: the warden parses shell grammar,
not Python semantics.

## Stamping the caller — per adapter

- **`caddis-warden.ts`**: the `CALLER` constant at the top of the adapter is
  stamped per agent so the shared ledger's `from:` field attributes every
  verdict to the caller that made the call. The onboard script stamps it for
  you: `./onboard my-agent-name`. An unstamped copy logs under the adapter's
  built-in default — safe to copy around.
- **`claude-code/caddis-warden-gate.py`**: the `from:` stamp comes from an
  optional `~/.caddis/lanes.json` cwd-prefix → label map (longest prefix wins,
  either path separator); unmapped sessions are stamped `claude-code`.

## Two behaviors worth knowing

- **Only written content is scanned**, never the text an edit replaces — in
  both adapters, for every tool shape: the warden must never punish you for
  removing the very thing it dislikes.
- **A steer arrives when it applies — per adapter**: the TS extension delivers
  it on the tool RESULT (the action was legitimate, it runs, and the law
  arrives attached to its outcome; the map of owed laws is keyed by tool-call
  id and deleted on delivery, so a long session cannot grow it). The Python
  hook delivers it as PreToolUse `additionalContext` — the same moment the
  decision is made, because a hook has no tool-result channel.

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
