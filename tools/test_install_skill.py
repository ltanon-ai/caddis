"""The onboarding skill install must be IDEMPOTENT.

`cp -r SRC DEST` copies INTO DEST when DEST already exists, so re-running
`onboard` — the documented path after every warden update — nested the fresh
skill at DEST/caddis/ and left the stale copy upstairs untouched, while still
printing "agent helper installed" and exiting 0. A success message over a no-op
is the exact failure this project exists against.

CI could never catch it: the self-proof only ever runs on a fresh runner, where
the destination does not yet exist. These tests run the SECOND install, which is
the one that was never exercised.
"""

import os
import shutil
import subprocess
from pathlib import Path

import pytest

SCRIPT = Path(__file__).resolve().parent / "install-skill.sh"

# Paths reach bash as POSIX text on every platform: Git Bash on Windows eats the
# backslashes of a native path, so a native string would make this suite measure
# something different there than it does on Linux and macOS.


def _bash():
    """A bash that can actually read this repository's paths.

    On a machine with WSL installed, `bash` on PATH is WSL's, and it cannot open
    a `C:/...` path at all — the suite would then fail for a reason that has
    nothing to do with the script under test. So candidates are PROBED rather
    than trusted by name, and a machine with no usable bash skips loudly instead
    of reporting a pass it never measured.
    """
    candidates = []
    if os.name == "nt":
        candidates.append(r"C:\Program Files\Git\bin\bash.exe")
    found = shutil.which("bash")
    if found:
        candidates.append(found)
    for candidate in candidates:
        if not Path(candidate).is_file():
            continue
        probe = subprocess.run(
            [candidate, "-c", 'test -f "' + SCRIPT.as_posix() + '"'],
            capture_output=True,
        )
        if probe.returncode == 0:
            return candidate
    pytest.skip("no bash on this machine can read the repository path")


def _make_src(tmp_path, marker="v1"):
    src = tmp_path / "skills" / "caddis"
    (src / "calibration").mkdir(parents=True, exist_ok=True)
    (src / "SKILL.md").write_text(marker + "\n", encoding="utf-8")
    (src / "ladder.py").write_text("print('" + marker + "')\n", encoding="utf-8")
    (src / "calibration" / "L1.md").write_text("card\n", encoding="utf-8")
    return src


def _install(src, *dests):
    return subprocess.run(
        [_bash(), SCRIPT.as_posix(), src.as_posix(), *[d.as_posix() for d in dests]],
        capture_output=True,
        text=True,
    )


def test_a_second_install_does_not_nest(tmp_path):
    src = _make_src(tmp_path)
    dest = tmp_path / "home" / ".claude" / "skills" / "caddis"
    assert _install(src, dest).returncode == 0
    assert _install(src, dest).returncode == 0
    assert not (dest / "caddis").exists(), "the re-run nested a copy one level deeper"
    assert (dest / "SKILL.md").is_file()


def test_a_second_install_refreshes_stale_content(tmp_path):
    src = _make_src(tmp_path, "v1")
    dest = tmp_path / "home" / ".claude" / "skills" / "caddis"
    _install(src, dest)
    _make_src(tmp_path, "v2")
    _install(src, dest)
    assert (dest / "SKILL.md").read_text(encoding="utf-8").strip() == "v2"
    assert "v2" in (dest / "ladder.py").read_text(encoding="utf-8")


def test_a_foreign_directory_is_left_alone(tmp_path):
    src = _make_src(tmp_path)
    dest = tmp_path / "somebody-elses"
    dest.mkdir()
    (dest / "their-file.txt").write_text("keep me\n", encoding="utf-8")
    result = _install(src, dest)
    assert (dest / "their-file.txt").is_file(), "a directory we cannot prove is ours was deleted"
    assert result.returncode != 0
    assert "WARNING" in result.stderr


def test_pycache_is_not_copied(tmp_path):
    src = _make_src(tmp_path)
    (src / "__pycache__").mkdir()
    (src / "__pycache__" / "ladder.pyc").write_bytes(b"\x00")
    dest = tmp_path / "home" / ".claude" / "skills" / "caddis"
    assert _install(src, dest).returncode == 0
    # Guarded: without this the assertion below would also hold for an install
    # that copied nothing at all, which is the very failure being tested for.
    assert (dest / "SKILL.md").is_file()
    assert not (dest / "__pycache__").exists()


def test_both_destinations_are_installed(tmp_path):
    src = _make_src(tmp_path)
    a = tmp_path / "home" / ".claude" / "skills" / "caddis"
    b = tmp_path / "home" / ".agents" / "skills" / "caddis"
    assert _install(src, a, b).returncode == 0
    assert (a / "SKILL.md").is_file() and (b / "SKILL.md").is_file()
