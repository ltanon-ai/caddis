"""warden_repl.py — the rlm nerve: the warden around rlm's exec surface.

rlm runs its model through a persistent IPython kernel, so every effect the
model causes is a PYTHON call — and the shell-shaped ones all funnel through
``subprocess.run/call/check_call/Popen`` and ``os.system/os.popen``. This
module wraps THAT surface: the standard library, never rlm's internals, so
no rlm version detail can break the nerve. ``install()`` replaces the six
entrypoints with wrappers that frame the command to the caddis-warden
binary and apply the verdict BEFORE the exec happens:

    deny  -> WardenRefusal(reason): the call never runs. The exception text
             IS the feedback channel — the kernel shows the model the same
             reason the operator would read in the ledger.
    steer -> the law lands on the stream (visible beside the output), the
             call RUNS.
    allow -> silent.

Wire spec: PROTOCOL.md in this repository
  request  <name> <byte-length>\\n<bytes>\\n  for tool, command, path, content
  response {"verdict":"allow|steer|deny","reason":...,"law":...,"seq":N}

THE FAILURE DOCTRINE, inherited verbatim from the claude-code nerve — the
two cases are deliberately opposite:
  binary missing / unspawnable -> ALLOW, screaming. A deployment problem
    must not brick the kernel at 3am — but a silently absent conscience is
    the exact failure this engine exists against.
  binary ran, reply unreadable -> REFUSE. It judged and we cannot read the
    judgement; trusting that is guessing. Judgement fails closed.

CALLER IDENTITY: every warden spawn carries CADDIS_WARDEN_FROM=rlm, so the
shared ledger answers "which agent did this" for rlm sessions.

⚠ THE BOUNDARY, stated where a reader meets it: this wraps SHELL exec. A
pure-Python destructive act with no subprocess call — shutil.rmtree(...),
open(path, "w"), os.remove — is OUT OF SCOPE, the same register as
THREAT-MODEL's embedded-program boundary: the warden parses shell grammar,
not Python semantics. Saying otherwise would be the unearned "verified"
this estate treats as its costliest failure.

Usage, from any rlm kernel cell (or sitecustomize):

    import warden_repl; warden_repl.install()

Removal (tests, operators): warden_repl.uninstall().
"""

from __future__ import annotations

import json
import os
import shlex
import subprocess
import sys
from pathlib import Path

TIMEOUT_S = 20
PREFIX = "[caddis-warden] "

_DEFAULT_BIN = (
    Path.home()
    / ".caddis"
    / "bin"
    / ("caddis-warden.exe" if os.name == "nt" else "caddis-warden")
)
# The TRUE baseline, captured at import and never reassigned: uninstall
# restores from HERE, not from the _ORIG_* names — those are the DELEGATE
# seam (tests fake them), and restoring a faked delegate into the live
# interpreter would corrupt every later subprocess call in the process.
_BASELINE = (
    subprocess.run,
    subprocess.call,
    subprocess.check_call,
    subprocess.Popen,
    os.system,
    os.popen,
)

# The REAL entrypoints, captured at import — the default delegates the
# wrappers call through, so wrapping can never recurse.
_ORIG_SUBPROCESS_RUN = subprocess.run
_ORIG_SUBPROCESS_CALL = subprocess.call
_ORIG_SUBPROCESS_CHECK_CALL = subprocess.check_call
_ORIG_SUBPROCESS_POPEN = subprocess.Popen
_ORIG_OS_SYSTEM = os.system
_ORIG_OS_POPEN = os.popen
_installed = False
_DELEGATES = {}
# The spawn the nerve ITSELF uses to reach the warden. Kept as its own name
# so a test (or an operator) can fake the binary side without touching the
# wrapped surface.
_ORIG_SPAWN_RUN = subprocess.run

_stream = sys.stderr


class WardenRefusal(RuntimeError):
    """The warden denied this exec; the message is the warden's reason."""


def _binary() -> Path:
    return Path(os.environ.get("CADDIS_WARDEN_BIN") or _DEFAULT_BIN)


def _frame(command: str) -> bytes:
    """Length-prefixed frame, fixed field order, BYTE counts (never chars).

    A byte count is what makes an arbitrary payload unable to break the
    frame. Path/content are empty: the exec surface carries only a command.
    """
    out = b""
    for name, value in (("tool", "bash"), ("command", command), ("path", ""), ("content", "")):
        raw = value.encode("utf-8")
        out += f"{name} {len(raw)}\n".encode() + raw + b"\n"
    return out


def _command_of(args) -> str | None:
    """The shell line this call would run, or None when it is not judgeable
    (a file object, a non-str/list argv) — those pass through, silently and
    honestly, rather than being guessed at."""
    if isinstance(args, str):
        return args
    if isinstance(args, (list, tuple)) and all(isinstance(a, str) for a in args):
        # shlex quoting keeps argv one parseable line: the joined string
        # reads back to the same words the shell would receive.
        return shlex.join(args)
    return None


def _scream(detail: str) -> None:
    _stream.write(PREFIX + f"binary absent ({detail}) — tools keep flowing, loudly\n")


def _ask(command: str) -> dict | None:
    """Frame one command to the warden. dict on a readable verdict; None
    when the binary is absent (allow-loud); WardenRefusal when it RAN but
    the verdict is unreadable (fail closed)."""
    binary = _binary()
    if not binary.exists():
        _scream(f"not found at {binary}")
        return None
    env = dict(os.environ)
    env["CADDIS_WARDEN_FROM"] = "rlm"
    try:
        proc = _ORIG_SPAWN_RUN(
            [str(binary)], input=_frame(command), capture_output=True,
            env=env, timeout=TIMEOUT_S,
        )
    except Exception as exc:  # unspawnable == a deployment problem
        _scream(f"could not spawn {binary}: {exc}")
        return None
    raw = (proc.stdout or b"").decode("utf-8", "replace").strip()
    try:
        reply = json.loads(raw)
        reply["verdict"]
        return reply
    except Exception:
        tail = raw[:200] or f"(empty stdout, rc={proc.returncode})"
        raise WardenRefusal(
            "caddis-warden: unreadable verdict — judgement fails closed. "
            f"Reply was: {tail}"
        )


def _judge(command: str) -> None:
    """Apply the warden's verdict to one command. Raises WardenRefusal on
    deny (and on unreadable); steers write the law and return; allow and
    absent-binary return silently."""
    reply = _ask(command)
    if reply is None:
        return
    verdict = str(reply.get("verdict"))
    if verdict == "deny":
        raise WardenRefusal(str(reply.get("reason") or "caddis-warden: denied"))
    if verdict == "steer":
        law = str(reply.get("law") or "").strip()
        reason = str(reply.get("reason") or "").strip()
        _stream.write(PREFIX + " ".join(x for x in (reason, law) if x) + "\n")
    if reply.get("seq") == 0:
        _stream.write("caddis-warden: seq 0 — decision NOT recorded in the ledger\n")


def _wrapped(name):
    """One wrapper: judge the command, then delegate THROUGH _DELEGATES —
    read at CALL time, so faking a delegate (tests, operators) works
    without rewiring the surface."""

    def wrapper(args, *rest, **kw):
        command = _command_of(args)
        if command is not None:
            _judge(command)
        return _DELEGATES[name](args, *rest, **kw)

    wrapper.__name__ = name
    return wrapper


def install(stream=None) -> None:
    """Wrap the exec surface. Idempotent. The optional stream is where
    steers and screams land (stderr by default)."""
    global _installed, _stream
    if stream is not None:
        _stream = stream
    if _installed:
        return
    _DELEGATES.update(
        run=_ORIG_SUBPROCESS_RUN,
        call=_ORIG_SUBPROCESS_CALL,
        check_call=_ORIG_SUBPROCESS_CHECK_CALL,
        Popen=_ORIG_SUBPROCESS_POPEN,
        system=_ORIG_OS_SYSTEM,
        popen=_ORIG_OS_POPEN,
    )
    subprocess.run = _wrapped("run")
    subprocess.call = _wrapped("call")
    subprocess.check_call = _wrapped("check_call")
    subprocess.Popen = _wrapped("Popen")
    os.system = _wrapped("system")
    os.popen = _wrapped("popen")
    _installed = True


def uninstall() -> None:
    """Restore the TRUE baseline entrypoints (tests, operators) — always
    the import-time originals, never whatever a delegate seam currently
    holds, so nothing a test faked can leak into the host process."""
    global _installed, _stream
    (
        subprocess.run,
        subprocess.call,
        subprocess.check_call,
        subprocess.Popen,
        os.system,
        os.popen,
    ) = _BASELINE
    _DELEGATES.clear()
    _stream = sys.stderr
    _installed = False
