"""test_warden_repl.py — the rlm nerve's invariants, tested without the
brain. Mirrors adapters/claude-code/test_warden_gate.py discipline: the
frame is byte-counted, the verdict application is pinned in all three
directions, the failure doctrine (absent binary allows loudly, unreadable
verdict refuses) is pinned, and the FROM stamp is checked where it is set.
"""

import importlib.util
import io
import os
import subprocess

_HERE = os.path.dirname(os.path.abspath(__file__))
_SPEC = importlib.util.spec_from_file_location(
    "warden_repl", os.path.join(_HERE, "warden_repl.py")
)
assert _SPEC is not None and _SPEC.loader is not None
repl = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(repl)


def _parse_frame(frame):
    out = {}
    i = 0
    while i < len(frame):
        nl = frame.index(b"\n", i)
        header = frame[i:nl].decode()
        name, length = header.split(" ")
        raw = frame[nl + 1 : nl + 1 + int(length)]
        assert frame[nl + 1 + int(length) : nl + 2 + int(length)] == b"\n"
        out[name] = raw.decode("utf-8")
        i = nl + 2 + int(length)
    return out


def test_frame_counts_bytes_not_chars():
    pairs = _parse_frame(repl._frame("git push --förce"))
    assert pairs["command"] == "git push --förce"
    raw = repl._frame("git push --förce")
    # ö is two bytes: the header length must say so (17, not 16)
    assert b"command 17\n" in raw


def test_frame_marks_the_tool_as_bash():
    assert _parse_frame(repl._frame("ls"))["tool"] == "bash"


def test_command_of_joins_list_args_shell_faithfully():
    assert repl._command_of("echo hi") == "echo hi"
    joined = repl._command_of(["rm", "-rf", "my dir"])
    assert joined == "rm -rf 'my dir'", "shlex quoting keeps argv one line"


def test_install_wraps_the_exec_surface_and_uninstall_restores():
    real = subprocess.run
    try:
        repl.install()
        assert subprocess.run is not real, "the exec surface is wrapped"
        assert os.system is not repl._ORIG_OS_SYSTEM
        repl.uninstall()
        assert subprocess.run is real
        assert os.system is repl._ORIG_OS_SYSTEM
    finally:
        repl.uninstall()


def test_install_is_idempotent():
    real = subprocess.run
    try:
        repl.install()
        once = subprocess.run
        repl.install()
        assert subprocess.run is once, "double install never double-wraps"
    finally:
        repl.uninstall()
    assert subprocess.run is real


def test_deny_refuses_the_exec_with_the_reason(monkeypatch):
    ran = []
    monkeypatch.setattr(repl, "_ORIG_SUBPROCESS_RUN", lambda *a, **k: ran.append(a))
    monkeypatch.setattr(
        repl, "_ask", lambda cmd: {"verdict": "deny", "reason": "caddis-warden [law.x]: no", "seq": 3}
    )
    stream = io.StringIO()
    repl.install(stream=stream)
    try:
        try:
            subprocess.run(["git", "push", "--force", "origin", "main"])
        except repl.WardenRefusal as ref:
            assert "law.x" in str(ref), "the refusal carries the warden reason"
        else:
            raise AssertionError("deny must refuse the exec")
        assert not ran, "the command never ran"
        assert stream.getvalue() == "", "deny refuses; it never also steers"
    finally:
        repl.uninstall()


def test_steer_runs_and_lands_the_law_beside_the_output(monkeypatch):
    ran = []
    monkeypatch.setattr(repl, "_ORIG_SUBPROCESS_RUN", lambda *a, **k: ran.append(a))
    monkeypatch.setattr(
        repl, "_ask",
        lambda cmd: {"verdict": "steer", "reason": "git.reset.discards-uncommitted", "law": "this discards 3 files", "seq": 4},
    )
    stream = io.StringIO()
    repl.install(stream=stream)
    try:
        subprocess.run(["git", "reset", "--hard"])
        assert ran, "steer lets the call through"
        assert "this discards 3 files" in stream.getvalue(), "the law lands beside the output"
    finally:
        repl.uninstall()


def test_allow_is_silent_and_runs(monkeypatch):
    ran = []
    monkeypatch.setattr(repl, "_ORIG_SUBPROCESS_RUN", lambda *a, **k: ran.append(a))
    monkeypatch.setattr(repl, "_ask", lambda cmd: {"verdict": "allow", "reason": "", "law": "", "seq": 5})
    stream = io.StringIO()
    repl.install(stream=stream)
    try:
        subprocess.run(["ls"])
        assert ran and stream.getvalue() == "", "allow: nothing to say"
    finally:
        repl.uninstall()


def test_absent_binary_allows_loudly(monkeypatch):
    ran = []
    monkeypatch.setattr(repl, "_ORIG_SUBPROCESS_RUN", lambda *a, **k: ran.append(a))
    monkeypatch.setattr(repl, "_binary", lambda: repl.Path("Z:/nowhere/caddis-warden.exe"))
    stream = io.StringIO()
    repl.install(stream=stream)
    try:
        subprocess.run(["ls"])
        assert ran, "a deployment problem must not brick the kernel"
        assert "not found" in stream.getvalue(), "but it is impossible to miss"
    finally:
        repl.uninstall()


def test_unreadable_verdict_refuses_the_exec(monkeypatch):
    ran = []
    monkeypatch.setattr(repl, "_ORIG_SUBPROCESS_RUN", lambda *a, **k: ran.append(a))

    def fake_run(binary_argv, **kw):
        class R:  # ran, but said nothing parseable
            stdout = b"not json"
            returncode = 0
        return R()

    monkeypatch.setattr(repl, "_ORIG_SPAWN_RUN", fake_run)
    stream = io.StringIO()
    repl.install(stream=stream)
    try:
        try:
            subprocess.run(["ls"])
        except repl.WardenRefusal as ref:
            assert "unreadable" in str(ref).lower(), "judgement fails closed"
        else:
            raise AssertionError("unreadable verdict must block")
        assert not ran, "nothing executed behind an unreadable judgement"
    finally:
        repl.uninstall()


def test_the_warden_spawn_carries_the_rlm_stamp(monkeypatch):
    seen = {}

    def fake_run(argv, **kw):
        seen["argv"] = argv
        seen["env"] = kw.get("env") or {}
        class R:
            stdout = b'{"verdict":"allow","reason":"","law":"","seq":9}'
            returncode = 0
        return R()

    monkeypatch.setattr(repl, "_ORIG_SPAWN_RUN", fake_run)
    repl._ask("ls")
    assert seen["env"].get("CADDIS_WARDEN_FROM") == "rlm", "the ledger row is attributed"


def test_judged_command_is_the_whole_frame_command(monkeypatch):
    asked = []
    monkeypatch.setattr(repl, "_ask", lambda cmd: (asked.append(cmd), {"verdict": "allow", "seq": 1})[1])
    monkeypatch.setattr(repl, "_ORIG_OS_SYSTEM", lambda cmd: 0)
    repl.install()
    try:
        os.system("rm -rf build")
        assert asked == ["rm -rf build"], "os.system is judged as the shell line"
    finally:
        repl.uninstall()

def test_the_real_binary_denies_a_destructive_command(monkeypatch):
    exe = "caddis-warden.exe" if os.name == "nt" else "caddis-warden"
    binary = None
    probe = _HERE
    for _ in range(4):  # canonical and projected trees differ in depth
        probe = os.path.dirname(probe)
        cand = os.path.join(probe, "target", "debug", exe)
        if os.path.isfile(cand):
            binary = cand
            break
    if binary is None:
        import pytest

        pytest.skip("workshop binary not built")
    ran = []
    monkeypatch.setattr(repl, "_ORIG_SUBPROCESS_RUN", lambda *a, **k: ran.append(a))
    monkeypatch.setattr(repl, "_binary", lambda: repl.Path(binary))
    repl.install()
    try:
        try:
            subprocess.run(["rm", "-rf", "/"])
        except repl.WardenRefusal as ref:
            assert "fs.rmrf.protected-root" in str(ref), f"the law names itself: {ref}"
        else:
            raise AssertionError("the real warden must deny rm -rf /")
        assert not ran, "nothing executed"
    finally:
        repl.uninstall()
