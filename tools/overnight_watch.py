#!/usr/bin/env python3
"""overnight_watch.py — the operator's night watchman.

Runs in a loop: checks all bee lanes, verifies completed work,
integrates, reports. Quality control: every completed card's Done-When
must pass before it's accepted. The operator sleeps; this doesn't.
"""
import json
import os
import re
import subprocess
import time
from pathlib import Path

REPO = Path(__file__).parent.parent
HOME = Path(os.environ.get("USERPROFILE") or os.environ.get("HOME", "."))
LINES = ["watch3", "watch4", "watch5"]
LOG = HOME / ".caddis" / "overnight.log"
CYCLE_S = 30
BEE_WINDOW_S = CYCLE_S * 2  # ~2 intervals = in-band freshness
_VERDICT = re.compile(r"^GATE \w+:\s+(CLEAN|\d+\s+VIOLATION)")
_prev_all_done = False  # CARD-KAT2: milestone fires on incomplete->complete TRANSITION only


def log(msg):
    ts = time.strftime("%H:%M:%S")
    line = f"[{ts}] {msg}"
    print(line)
    with open(LOG, "a") as f:
        f.write(line + "\n")


def queue_head(lineage):
    q = HOME / ".caddis" / "rotation" / "lines" / lineage / "queue"
    if not q.exists():
        return None
    for line in q.read_text().splitlines():
        line = line.strip()
        if line and not line.startswith("#") and not line.startswith("done"):
            return line.split()[0]  # CARD-XXXX
    return None


def done_count(lineage):
    q = HOME / ".caddis" / "rotation" / "lines" / lineage / "queue"
    if not q.exists():
        return 0
    return sum(1 for l in q.read_text().splitlines()
               if l.strip().startswith("done CARD-"))


def gate_check():
    """Run the fast gate; parse the VERDICT LINE, detail from VIOLATION lines."""
    r = subprocess.run(
        ["python", "tools/gate.py", "--fast"],
        cwd=REPO, capture_output=True, text=True, timeout=120
    )
    clean = False
    detail = ""
    for line in r.stdout.splitlines():
        m = _VERDICT.match(line)
        if m:
            clean = m.group(1) == "CLEAN"
        elif line.startswith("GATE VIOLATION"):
            detail = (detail + "; " + line) if detail else line
    return clean, detail


def cargo_test():
    """Full workspace test — the ultimate quality gate."""
    r = subprocess.run(
        ["cargo", "test", "--workspace"],
        cwd=REPO, capture_output=True, text=True, timeout=600
    )
    failed = r.returncode != 0
    count_ok = r.stdout.count("test result: ok")
    count_fail = r.stdout.count("test result: FAILED")
    return not failed, count_ok, count_fail


def _recent_bee(lineage):
    """True if bee.log's last entry is within ~2 intervals (just finished)."""
    p = HOME / ".caddis" / "rotation" / "lines" / lineage / "bee.log"
    if not p.exists():
        return False
    try:
        last = p.read_text().splitlines()[-1]
        ts = int(json.loads(last)["ts"])
    except (ValueError, IndexError, KeyError, json.JSONDecodeError):
        return False
    return (time.time() - ts) <= BEE_WINDOW_S


def _keeper_heartbeat(lineage):
    """CARD-0262: True if keeper.heartbeat mtime is within ~2 intervals."""
    p = HOME / ".caddis" / "rotation" / "lines" / lineage / "keeper.heartbeat"
    if not p.exists():
        return False
    try:
        mtime = p.stat().st_mtime
    except OSError:
        return False
    return (time.time() - mtime) <= BEE_WINDOW_S


def _pace_working(lineage):
    """True if pace.line carries a fresh PACE WORK sentence (mid-card)."""
    p = HOME / ".caddis" / "rotation" / "lines" / lineage / "pace.line"
    if not p.exists():
        return False
    sentence = ""
    ts = 0
    for line in p.read_text().splitlines():
        if line.startswith("sentence="):
            sentence = line.split("=", 1)[1]
        elif line.startswith("ts="):
            try:
                ts = int(line.split("=", 1)[1])
            except ValueError:
                pass
    if "WORK" not in sentence:
        return False
    return 0 < ts and (time.time() - ts) <= BEE_WINDOW_S


def bee_alive(lineage):
    """Detect the in-band beekeeper working state, not a phantom process.

    No queued head = between cycles (alive). A queued head with recent
    bee.log activity OR a fresh PACE WORK line OR a fresh
    keeper.heartbeat = mid-card (alive). A queued head silent >2
    intervals (stale bee.log AND no fresh PACE WORK AND no fresh
    heartbeat) = DEAD truthfully.
    """
    if queue_head(lineage) is None:
        return True
    if _recent_bee(lineage):
        return True
    if _pace_working(lineage):
        return True
    if _keeper_heartbeat(lineage):
        return True
    return False


def _any_bee_working():
    """True if ANY lane has a queued head and a live bee (mid-card)."""
    return any(queue_head(l) is not None and bee_alive(l) for l in LINES)


def main():
    log("=== OVERNIGHT WATCH START ===")
    cycle = 0
    while True:
        cycle += 1
        log(f"--- Cycle {cycle} ---")

        # 1. Check each lane
        for lineage in LINES:
            card = queue_head(lineage)
            done = done_count(lineage)
            alive = bee_alive(lineage)
            status = "WORKING" if alive else "DEAD"
            log(f"  {lineage}: head={card or 'IDLE'} done={done} bee={status}")

        if cycle % 10 == 0:
            periodic_gate(cycle)
        if cycle % 30 == 0:
            periodic_tests(cycle)
        check_all_done()

        time.sleep(CYCLE_S)


def periodic_gate(cycle):
    gate_ok, gate_detail = gate_check()
    log(f"  GATE: {'CLEAN' if gate_ok else 'VIOLATION: ' + gate_detail}")
    if gate_ok:
        return
    if _any_bee_working():
        log("  red gate with bee mid-card — in-flight, not filing")
        return
    log("  !! Gate violation — filing blocker")
    blockers = HOME / ".caddis" / "rotation" / "lines" / "watch3" / "blockers.jsonl"
    blockers.parent.mkdir(parents=True, exist_ok=True)
    with open(blockers, "a") as f:
        f.write(json.dumps({
            "source": "overnight-watch",
            "reason": f"gate violation cycle {cycle}: {gate_detail[:100]}",
            "ts": time.strftime("%Y-%m-%dT%H:%M:%SZ")
        }) + "\n")


def periodic_tests(cycle):
    tests_ok, ok_count, fail_count = cargo_test()
    log(f"  TESTS: {ok_count} ok, {fail_count} FAILED")
    if not tests_ok:
        with open(HOME / ".caddis" / "overnight-test-failures.log", "a") as f:
            f.write(f"[{time.strftime('%H:%M:%S')}] cycle {cycle}: {fail_count} failures\n")


def check_all_done():
    """Run final verification EVERY cycle; log the milestone ONCE per transition.

    The watch runs until process death / operator stop. A green banner
    is a milestone, not an exit — and a milestone is a state TRANSITION
    (CARD-KAT2): the ALL-COMPLETE line logs on incomplete->complete only.
    Steady-complete cycles stay silent but still run tests+gate; a red
    result always speaks (FINAL + Issues detected on every failing cycle).
    """
    global _prev_all_done
    all_done = all(queue_head(l) is None for l in LINES)
    is_transition = all_done and not _prev_all_done
    _prev_all_done = all_done  # a queued card re-arms the milestone
    if not all_done:
        return False
    if is_transition:
        log("  ALL CARDS COMPLETE — final verification")
    tests_ok, ok_count, fail_count = cargo_test()
    gate_ok, _ = gate_check()
    green = tests_ok and gate_ok
    if is_transition or not green:
        log(f"  FINAL: tests={ok_count}ok/{fail_count}fail gate={'CLEAN' if gate_ok else 'DIRTY'}")
    if green:
        if is_transition:
            log("  === ALL GREEN — overnight mission complete ===")
    else:
        log("  Issues detected — continuing watch")
    return False


if __name__ == "__main__":
    main()