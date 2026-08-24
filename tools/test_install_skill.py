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


def test_zero_destinations_is_a_usage_error(tmp_path):
    src = _make_src(tmp_path)
    result = subprocess.run(
        [_bash(), SCRIPT.as_posix(), src.as_posix()], capture_output=True, text=True
    )
    assert result.returncode == 2, "a call that installs nowhere must not read as success"


def test_the_source_is_never_installed_over_itself(tmp_path):
    # The destination is removed before the copy, so a destination that resolves
    # to the source would delete the very tree being installed.
    src = _make_src(tmp_path)
    result = _install(src, src)
    assert (src / "SKILL.md").is_file(), "the source tree was destroyed"
    assert result.returncode != 0


def test_a_partial_failure_still_installs_the_good_destination(tmp_path):
    src = _make_src(tmp_path)
    foreign = tmp_path / "somebody-elses"
    foreign.mkdir()
    (foreign / "their-file.txt").write_text("keep me\n", encoding="utf-8")
    good = tmp_path / "home" / ".claude" / "skills" / "caddis"
    result = _install(src, foreign, good)
    assert (good / "SKILL.md").is_file()
    assert (foreign / "their-file.txt").is_file()
    assert "WARNING" in result.stderr, "the refused destination must be reported"
    assert result.returncode == 0, "one good install is still an install"


# ── the sourced invocation ──────────────────────────────────────────────────
#
# The GitHub matrix runs `. ./onboard`. Sourcing does NOT change `$0`, so a
# script that locates its own files through `$0` reaches for the SOURCING
# script's directory instead of its own — and the skill install quietly went
# looking outside the repository. These tests run the real `onboard` from a
# synthetic repo so they stay fast and portable.

REPO = Path(__file__).resolve().parents[1]
NL = chr(10)


def _fake_repo(tmp_path):
    """A minimal repo layout: onboard, its helper, a skill, and the binary."""
    onboard = REPO / "onboard"
    tools = REPO / "tools" / "install-skill.sh"
    if not onboard.is_file() or not tools.is_file():
        pytest.skip("onboard and its helper are not beside this test")
    exe = None
    for root in (REPO, REPO.parent):
        for name in ("caddis-warden.exe", "caddis-warden"):
            candidate = root / "target" / "release" / name
            if candidate.is_file():
                exe = candidate
                break
        if exe:
            break
    if exe is None:
        pytest.skip("no release binary built; nothing to onboard")

    repo = tmp_path / "repo"
    (repo / "tools").mkdir(parents=True)
    (repo / "target" / "release").mkdir(parents=True)
    (repo / "skills" / "caddis" / "calibration").mkdir(parents=True)
    shutil.copy(onboard, repo / "onboard")
    shutil.copy(tools, repo / "tools" / "install-skill.sh")
    shutil.copy(exe, repo / "target" / "release" / exe.name)
    (repo / "skills" / "caddis" / "SKILL.md").write_text("skill" + NL, encoding="utf-8")
    (repo / "skills" / "caddis" / "ladder.py").write_text("x = 1" + NL, encoding="utf-8")

    # `cargo build --release` is not what these tests measure, and building here
    # would make them minutes long: a shim satisfies onboard's build step.
    shim_dir = tmp_path / "shim"
    shim_dir.mkdir()
    shim = shim_dir / "cargo"
    shim.write_text("#!/bin/sh" + NL + "exit 0" + NL, encoding="utf-8")
    shim.chmod(0o755)
    return repo, shim_dir


def _run_onboard(tmp_path, repo, shim_dir, sourced):
    home = tmp_path / "home"
    (home / ".caddis").mkdir(parents=True)
    env = dict(os.environ)
    env["HOME"] = str(home)
    env["PATH"] = str(shim_dir) + os.pathsep + env.get("PATH", "")
    env["CADDIS_WARDEN_LEDGER"] = (home / ".caddis" / "warden-ledger.jsonl").as_posix()
    if sourced:
        # Deliberately in ANOTHER directory: that is what makes $0 wrong.
        sourcer = tmp_path / "elsewhere" / "sourcer.sh"
        sourcer.parent.mkdir(parents=True, exist_ok=True)
        sourcer.write_text(". ./onboard e2e" + NL, encoding="utf-8")
        argv = [_bash(), sourcer.as_posix()]
    else:
        argv = [_bash(), (repo / "onboard").as_posix(), "e2e"]
    result = subprocess.run(
        argv, cwd=str(repo), env=env, capture_output=True, text=True
    )
    return result, home / ".claude" / "skills" / "caddis" / "SKILL.md"


def test_onboard_installs_the_skill_when_sourced(tmp_path):
    repo, shim = _fake_repo(tmp_path)
    result, skill = _run_onboard(tmp_path, repo, shim, sourced=True)
    assert skill.is_file(), (
        "sourcing left the skill uninstalled — $0 is the sourcing script, "
        f"not onboard.{NL}{result.stdout}{result.stderr}"
    )
    assert result.returncode == 0


def test_onboard_installs_the_skill_when_executed(tmp_path):
    repo, shim = _fake_repo(tmp_path)
    result, skill = _run_onboard(tmp_path, repo, shim, sourced=False)
    assert skill.is_file(), result.stdout + result.stderr
    assert result.returncode == 0


def test_a_destination_whose_parent_cannot_be_made_is_reported(tmp_path):
    src = _make_src(tmp_path)
    blocker = tmp_path / "wall"
    blocker.write_text("i am a file, not a directory" + NL, encoding="utf-8")
    result = _install(src, blocker / "skills" / "caddis")
    assert result.returncode != 0
    assert "WARNING" in result.stderr
