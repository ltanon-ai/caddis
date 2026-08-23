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
    assert gate._marshal("", {"command": "x"}) == ("unknown", "x", "", "")


def test_longest_lane_prefix_wins(monkeypatch):
    monkeypatch.setattr(gate, "_lanes", lambda: {"/a": "short", "/a/b": "longest"})
    assert gate._longest_lane_match("/a/b/c") == "longest"
    assert gate._longest_lane_match("/a") == "short"
    assert gate._longest_lane_match("/ab/c") == ""  # prefix must end at a separator
    assert gate._longest_lane_match("") == ""


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
