"""test_warden_gate.py — the nerve's invariants, tested without the brain.

The Claude Code adapter is a thin marshaller: tool shapes, the frame's
BYTE-count prefix, lane attribution. These tests pin the properties the
warden's protocol depends on — most importantly THE ONE RULE WITH TEETH:
an edit's old_string is never marshalled, so the warden can never punish
you for cleaning up the very thing it dislikes.
"""

import importlib.util
import os

_HERE = os.path.dirname(os.path.abspath(__file__))
_SPEC = importlib.util.spec_from_file_location(
    "caddis_warden_gate", os.path.join(_HERE, "caddis-warden-gate.py")
)
assert _SPEC is not None and _SPEC.loader is not None
gate = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(gate)


def _parse_frame(frame):
    """Split a length-prefixed frame back into (name, payload) pairs."""
    out = []
    i = 0
    while i < len(frame):
        j = frame.index(b"\n", i)
        name, count = frame[i:j].decode().split(" ")
        i = j + 1
        out.append((name, frame[i : i + int(count)]))
        i += int(count) + 1  # payload + the trailing newline
    return out


def test_frame_counts_bytes_not_chars():
    pairs = dict(_parse_frame(gate._frame("bash", "git push --förce", "p", "héllo")))
    assert pairs["command"] == "git push --förce".encode("utf-8")
    assert pairs["content"] == "héllo".encode("utf-8")
    assert len(pairs["command"]) == 17  # ö is two bytes, not one char


def test_frame_roundtrips_every_field_in_order():
    pairs = _parse_frame(gate._frame("edit", "cmd", "path", "content"))
    assert [name for name, _ in pairs] == ["tool", "command", "path", "content"]


def test_shape_shell_sends_command_only():
    shape = gate._shape_shell("bash", {"command": "ls", "file_path": "ignored"})
    assert shape == ("bash", "ls", "", "")


def test_shape_write_reads_file_path_and_content():
    shape = gate._shape_write("Write", {"file_path": "a.py", "content": "x = 1"})
    assert shape == ("write", "", "a.py", "x = 1")


def test_edit_never_marshals_the_replaced_text():
    ti = {"file_path": "a.py", "old_string": "SECRET-OLD", "new_string": "clean"}
    shape = gate._shape_edit("Edit", ti)
    assert "SECRET-OLD" not in shape
    assert b"SECRET-OLD" not in gate._frame(*shape)  # THE ONE RULE WITH TEETH


def test_notebook_shape_reads_new_source():
    shape = gate._shape_notebook(
        "NotebookEdit", {"notebook_path": "n.ipynb", "new_source": "print(1)"}
    )
    assert shape == ("edit", "", "n.ipynb", "print(1)")


def test_unknown_tool_falls_back_to_the_generic_shape():
    shape = gate._marshal("Grep", {"command": "rg x", "file_path": "a", "content": "b"})
    assert shape == ("grep", "rg x", "a", "b")


def test_marshal_normalizes_names_and_defaults_the_empty_one():
    assert gate._marshal("  Bash ", {"command": "ls"})[0] == "bash"


def test_windows_separators_match_forward_slash_lanes(monkeypatch):
    monkeypatch.setattr(gate, "_lanes", lambda: {"C:/proj": "owl"})
    assert gate._longest_lane_match("C:\\proj\\src") == "owl"
    assert gate._longest_lane_match("C:/proj/src") == "owl"
    assert gate._longest_lane_match("C:\\proj") == "owl"
    assert gate._longest_lane_match("C:\\projects") == ""


def test_mixed_separators_and_trailing_slashes_match(monkeypatch):
    monkeypatch.setattr(gate, "_lanes", lambda: {"C:\\lab\\": "lab"})
    assert gate._longest_lane_match("C:/lab/goal") == "lab"
    assert gate._longest_lane_match("C:\\lab\\goal\\") == "lab"


def test_posix_paths_stay_case_sensitive(monkeypatch):
    # normcase is the identity on POSIX; simulate it so the guarantee is
    # pinned even when the suite runs on Windows.
    monkeypatch.setattr(gate.os.path, "normcase", lambda s: s)
    monkeypatch.setattr(gate, "_lanes", lambda: {"/home/proj": "owl"})
    assert gate._longest_lane_match("/home/proj/src") == "owl"
    assert gate._longest_lane_match("/Home/proj/src") == "", "POSIX case is real"


def test_windows_case_folds_where_the_filesystem_does(monkeypatch):
    # normcase lowercases and backslash-folds on Windows; simulate it so
    # the guarantee is pinned even when the suite runs on POSIX cells.
    monkeypatch.setattr(
        gate.os.path, "normcase", lambda s: s.replace("/", "\\").lower()
    )
    monkeypatch.setattr(gate, "_lanes", lambda: {"C:/proj": "owl"})
    assert gate._longest_lane_match("C:\\PROJ\\src") == "owl"


def test_a_root_prefix_never_becomes_a_catch_all(monkeypatch):
    monkeypatch.setattr(
        gate, "_lanes", lambda: {"/": "all", "C:/": "drive", "/real": "real"}
    )
    assert gate._longest_lane_match("/anything/at/all") == ""
    assert gate._longest_lane_match("C:\\Windows\\foo") == "", "drive root is no lane"
    assert gate._longest_lane_match("/real/work") == "real"


def test_label_falls_back_to_the_generic_stamp(monkeypatch):
    monkeypatch.setattr(gate, "_lanes", lambda: {})
    monkeypatch.delenv("CADDIS_WARDEN_STAND_ASIDE", raising=False)
    assert gate._label_for("/nowhere") == "claude-code"


def test_label_stand_aside_yields_the_empty_stamp(monkeypatch):
    monkeypatch.setattr(gate, "_lanes", lambda: {})
    monkeypatch.setenv("CADDIS_WARDEN_STAND_ASIDE", "1")
    assert gate._label_for("/nowhere") == ""


def test_label_uses_the_lane_when_one_matches(monkeypatch):
    monkeypatch.setattr(gate, "_lanes", lambda: {"/proj": "owl"})
    monkeypatch.delenv("CADDIS_WARDEN_STAND_ASIDE", raising=False)
    assert gate._label_for("/proj/src") == "owl"


def test_session_scoped_stamp_distinguishes_two_sessions_in_one_lane(monkeypatch):
    """CARD-0109: the card gate keys on `from`, so a lane label alone would let
    one session's card bound another session's writes."""
    assert gate._session_scoped("peleda", "a1b2c3d4e5f6") == "peleda.a1b2c3d4"
    assert gate._session_scoped("peleda", "0000000000") != gate._session_scoped(
        "peleda", "1111111111"
    )


def test_a_missing_session_id_degrades_to_todays_stamp(monkeypatch):
    """Absent is not invented. A fabricated per-process id would look
    session-scoped to every reader while changing on every single tool call,
    because the warden is spawned once per call."""
    assert gate._session_scoped("peleda", "") == "peleda"
    assert gate._session_scoped("claude-code", "") == "claude-code"
    # Stand-aside keeps yielding the empty stamp rather than a bare dot.
    assert gate._session_scoped("", "a1b2c3d4") == ""


def test_the_session_suffix_cannot_corrupt_the_ledger_row(monkeypatch):
    """`caller_id()` keeps 32 chars and drops anything outside
    [A-Za-z0-9_.-]; a hostile session id must not smuggle a separator through
    or push the label out of the window."""
    assert gate._session_scoped("peleda", '"|x\n') == "peleda.x"
    assert gate._session_scoped("peleda", "../../etc") == "peleda.etc"
    stamped = gate._session_scoped("peleda", "0123456789abcdef")
    assert stamped == "peleda.01234567", stamped
    assert len(stamped) <= 32


def test_the_spawned_process_is_handed_the_session_scoped_caller(monkeypatch):
    """Asserted on the env actually handed to the child, not on a return
    value: the stamp only matters if it reaches the warden."""
    seen = {}

    class _Proc:
        stdout = b'{"verdict":"allow"}'
        stderr = b""

    def _fake_run(argv, **kw):
        seen["env"] = kw.get("env") or {}
        return _Proc()

    monkeypatch.setattr(gate.subprocess, "run", _fake_run)
    monkeypatch.setattr(gate.Path, "exists", lambda self: True)
    gate._ask_warden(gate.Path("warden"), b"", "peleda.a1b2c3d4")
    assert seen["env"].get("CADDIS_WARDEN_FROM") == "peleda.a1b2c3d4"
