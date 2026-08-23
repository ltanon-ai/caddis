"""PreToolUse adapter for the caddis-warden binary — the nerve, not the brain.

Marshals a Claude Code tool call into the caddis wire frame, spawns the binary,
applies the verdict. It holds NO policy: every judgement belongs to the warden,
so this file never grows a rule of its own.

Wire spec: PROTOCOL.md in this repository
  request  <name> <byte-length>\n<bytes>\n  for tool, command, path, content
  response {"verdict":"allow|steer|deny","reason":...,"law":...,"seq":N}

Register in Claude Code's settings (hooks.PreToolUse), matcher: "*"::

    {
      "hooks": {
        "PreToolUse": [
          { "matcher": "*",
            "hooks": [ { "type": "command",
                         "command": "python ~/.claude/hooks/caddis-warden-gate.py" } ] }
        ]
      }
    }

THE FAILURE DOCTRINE, and the two cases are deliberately opposite:
  binary missing / unspawnable -> ALLOW, screaming. A deployment problem must
    not brick the agent at 3am -- but a silently absent conscience is the exact
    failure this engine exists against, so it is made impossible to miss.
  binary ran, reply unreadable  -> BLOCK. It judged and we cannot read the
    judgement; trusting that is guessing. Judgement fails closed.

CALLER IDENTITY — one conscience, many bodies, each body named:
  The `from:` stamp in the shared ledger comes from an optional lane map at
  ~/.caddis/lanes.json — a JSON object of {"<cwd-prefix>": "<label>", ...},
  longest prefix wins. Sessions under no mapped prefix are stamped the
  generic "claude-code". Fleets that deliberately wire only some directories
  can set CADDIS_WARDEN_STAND_ASIDE=1: unmapped sessions then stand aside
  silently (no judgement, no ledger row) — a deliberate act, never a default.

Lineage: first authored by a Claude Code session on its own onboarding day
(the fourth harness to join the conscience), genericized here for everyone.
"""

from __future__ import annotations

import json
import os
import subprocess
import sys
from pathlib import Path

TIMEOUT_S = 20

_DEFAULT_BIN = (
    Path.home()
    / ".caddis"
    / "bin"
    / ("caddis-warden.exe" if os.name == "nt" else "caddis-warden")
)


def _lanes() -> dict:
    """Optional cwd-prefix -> label map from ~/.caddis/lanes.json."""
    path = Path.home() / ".caddis" / "lanes.json"
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
        return (
            {str(k): str(v) for k, v in data.items()} if isinstance(data, dict) else {}
        )
    except Exception:
        return {}


def _label_for(cwd: str) -> str:
    """The `from:` stamp for this session. Never empty: an unnamed body is
    still a body, and an audit that cannot say WHO did a thing is the exact
    failure the `from:` field exists against."""
    match = _longest_lane_match(cwd)
    if match:
        return match
    if os.environ.get("CADDIS_WARDEN_STAND_ASIDE") == "1":
        return ""
    return "claude-code"


def _longest_lane_match(cwd: str) -> str:
    if not cwd:
        return ""
    norm = cwd.replace("/", os.sep).rstrip(os.sep).lower()
    best, best_len = "", -1
    for prefix, label in _lanes().items():
        p = prefix.rstrip(os.sep).lower()
        if (norm == p or norm.startswith(p + os.sep)) and len(p) > best_len:
            best, best_len = label, len(p)
    return best


def _binary() -> Path:
    return Path(os.environ.get("CADDIS_WARDEN_BIN") or _DEFAULT_BIN)


def _frame(tool: str, command: str, path: str, content: str) -> bytes:
    """Length-prefixed frame, fixed field order, BYTE counts (never chars).

    A byte count is what makes an arbitrary payload unable to break the frame.
    """
    out = b""
    for name, value in (
        ("tool", tool),
        ("command", command),
        ("path", path),
        ("content", content),
    ):
        raw = (value or "").encode("utf-8")
        out += f"{name} {len(raw)}\n".encode() + raw + b"\n"
    return out


# ── tool marshalling — dispatch table, one extractor per tool shape ─────────
#
# THE ONE RULE WITH TEETH lives here as a property of every extractor: for
# edits only the text being WRITTEN is scanned, never the text being replaced
# -- the warden must never punish you for cleaning up the very thing it
# dislikes. So `old_string` is deliberately never sent by any shape.


def _shape_shell(name: str, ti: dict) -> tuple[str, str, str, str]:
    return (name, str(ti.get("command") or ""), "", "")


def _shape_write(_name: str, ti: dict) -> tuple[str, str, str, str]:
    return ("write", "", str(ti.get("file_path") or ""), str(ti.get("content") or ""))


def _shape_edit(_name: str, ti: dict) -> tuple[str, str, str, str]:
    return ("edit", "", str(ti.get("file_path") or ""), str(ti.get("new_string") or ""))


def _shape_notebook(_name: str, ti: dict) -> tuple[str, str, str, str]:
    return (
        "edit",
        "",
        str(ti.get("notebook_path") or ""),
        str(ti.get("new_source") or ""),
    )


def _shape_generic(name: str, ti: dict) -> tuple[str, str, str, str]:
    # Unknown tool: send what is there rather than inventing a shape.
    return (
        name,
        str(ti.get("command") or ""),
        str(ti.get("file_path") or ""),
        str(ti.get("content") or ""),
    )


_TOOL_SHAPES = {
    "bash": _shape_shell,
    "powershell": _shape_shell,
    "write": _shape_write,
    "edit": _shape_edit,
    "notebookedit": _shape_notebook,
}


def _marshal(tool_name: str, ti: dict) -> tuple[str, str, str, str]:
    name = (tool_name or "").strip().lower() or "unknown"
    return _TOOL_SHAPES.get(name, _shape_generic)(name, ti)


def _emit_block(reason: str) -> None:
    sys.stdout.write(json.dumps({"decision": "block", "reason": reason}))


def _emit_context(text: str) -> None:
    sys.stdout.write(
        json.dumps(
            {
                "hookSpecificOutput": {
                    "hookEventName": "PreToolUse",
                    "additionalContext": text,
                }
            }
        )
    )


def _scream_absent(detail: str) -> None:
    """Allow, but make the missing conscience impossible to miss."""
    msg = f"CONSCIENCE OFFLINE — {detail}. Tools are running UNJUDGED."
    sys.stderr.write("[caddis-warden] " + msg + "\n")
    _emit_context("[caddis-warden] " + msg)


def _read_event() -> dict | None:
    """Parse the hook payload; block on an unreadable one (our parse failure
    must not certify a command as safe)."""
    try:
        return json.loads(sys.stdin.read() or "{}")
    except Exception:
        _emit_block("caddis-warden adapter: unreadable hook payload")
        return None


def _ask_warden(binary: Path, frame: bytes, caller: str) -> dict:
    """Spawn the binary and return its verdict dict, or {} when the reply
    could not be read. Applies the failure doctrine: unspawnable allows loudly
    (via _scream_absent and {}), unreadable blocks (via _emit_block)."""
    if not binary.exists():
        _scream_absent(f"binary not found at {binary}")
        return {}
    env = dict(os.environ)
    env["CADDIS_WARDEN_FROM"] = caller
    try:
        proc = subprocess.run(
            [str(binary)], input=frame, capture_output=True, env=env, timeout=TIMEOUT_S
        )
    except Exception as exc:  # unspawnable == a deployment problem
        _scream_absent(f"could not spawn {binary}: {exc}")
        return {}
    raw = (proc.stdout or b"").decode("utf-8", "replace").strip()
    try:
        reply = json.loads(raw)
        reply["verdict"]
        return reply
    except Exception:
        # It RAN and judged; we cannot read the judgement. Fail closed.
        tail = raw[:200] or f"(empty stdout, rc={proc.returncode})"
        _emit_block(
            "caddis-warden: unreadable verdict — judgement fails closed. "
            f"Reply was: {tail}"
        )
        return {}


def _apply(reply: dict) -> None:
    if reply.get("seq") == 0:
        sys.stderr.write("caddis-warden: seq 0 — decision NOT recorded in the ledger\n")
    verdict = str(reply["verdict"])
    if verdict == "deny":
        _emit_block(str(reply.get("reason") or "caddis-warden: denied"))
    elif verdict == "steer":
        law = str(reply.get("law") or "").strip()
        reason = str(reply.get("reason") or "").strip()
        _emit_context("[caddis-warden] " + " ".join(x for x in (reason, law) if x))


def main() -> int:
    data = _read_event()
    if data is None:
        return 0

    caller = _label_for(str(data.get("cwd") or ""))
    if not caller:
        return 0  # stand-aside mode: deliberately unwired, no ledger row

    tool, command, path, content = _marshal(
        data.get("tool_name") or "", data.get("tool_input") or {}
    )
    reply = _ask_warden(_binary(), _frame(tool, command, path, content), caller)
    if reply:
        _apply(reply)
    return 0


if __name__ == "__main__":
    try:
        sys.exit(main())
    except Exception as exc:  # never let the nerve kill the session
        sys.stderr.write(f"[caddis-warden] adapter internal error: {exc}\n")
        sys.exit(0)
