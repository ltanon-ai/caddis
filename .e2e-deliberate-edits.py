#!/usr/bin/env python3
"""E2E: caddis-deliberate `edits` CLI (P4 s4a) against the RELEASE binary.

Proves the verb surface the P4 world bridge will ride:
  status   — honest census, rc0 on an absent journal (exists:false)
  propose  --op/--card, durable pending, prior16 == external sha256[..16]
  confirm  — GateClosed rc1 with an absent ledger (nothing written), then
             CONFIRMED through a card.open ledger row (body law), stream
             FIRST + view resync, re-propose = no-op rc1, stale rc1 after
             the stream moves, NotPending rc1 after refuse, unknown rc1
  refuse   — the operator's NO, journaled (status refused count)
  usage    — unknown flag / missing values / multi-card / op-word mismatch rc2
  REAL home read-only smoke — edits.jsonl absent => empty honest census.

Sandbox seeds copy REAL card lines from ~/.caddis/deliberate/seats.jsonl
(read-only) so the grammar under test is the organ's own, never hand-typed.
"""
import hashlib
import json
import os
import subprocess
import sys
import tempfile

ROOT = os.path.dirname(os.path.abspath(__file__))
BIN = os.path.join(ROOT, "target", "release", "caddis-deliberate.exe")
REAL_STREAM = os.path.join(os.environ.get("USERPROFILE", "."), ".caddis", "deliberate", "seats.jsonl")

PASS = 0
FAIL = 0


def check(name, cond, detail=""):
    global PASS, FAIL
    if cond:
        PASS += 1
        print(f"  ok   {name}")
    else:
        FAIL += 1
        print(f"  FAIL {name} :: {detail}")


def run(*args):
    r = subprocess.run([BIN, *args], capture_output=True, text=True, timeout=60)
    return r.returncode, r.stdout, r.stderr


def jout(stdout):
    return json.loads(stdout.strip().splitlines()[-1])


def prior16(path):
    with open(path, "rb") as f:
        return hashlib.sha256(f.read()).hexdigest()[:16]


def stream_lines(path):
    with open(path, "r", encoding="utf-8") as f:
        return [l for l in f.read().splitlines() if l.strip()]


def warden_open_row(actor, card_id):
    # Tight separators: the warden row parser extracts "seq":N literally —
    # spaced JSON ("seq": 1) is UNREADABLE to it (model-voice organ lesson).
    return json.dumps({
        "seq": 1, "v": 1, "id": "x", "idem_key": "k", "type": "card.open",
        "from": actor, "to": "warden",
        "body": f"open|{card_id}|_card_x.md|deadbeef", "ts": "1",
    }, separators=(",", ":"))

def card_keys(line):
    try:
        return set(json.loads(line))
    except Exception:
        return set()


def main():
    with open(REAL_STREAM, "r", encoding="utf-8") as f:
        real = [l for l in f.read().splitlines() if l.strip()]
    seat_line = next(l for l in real if "provider" in card_keys(l))
    extra_line = real[3]
    sbx = tempfile.mkdtemp(prefix="caddis-edits-e2e-")
    home = os.path.join(sbx, "home")
    os.makedirs(home)
    stream = os.path.join(home, "seats.jsonl")
    journal = os.path.join(home, "edits.jsonl")
    with open(stream, "w", encoding="utf-8", newline="\n") as f:
        f.write("\n".join(real[:3]) + "\n")

    print("== bootstrap: view syncs the sandbox ==")
    rc, _, _ = run("view", "--home", home)
    check("view rc0 on the seeded sandbox", rc == 0)

    print("== status: honest empty census ==")
    rc, out, _ = run("edits", "status", "--home", home)
    check("status rc0 with absent journal", rc == 0)
    st = jout(out)
    check("status shape", set(st) == {"version", "journal", "exists", "max_seq",
                                      "pending", "confirmed", "refused", "unparseable"},
          f"keys={sorted(st)}")
    check("empty census", st["pending"] == [] and st["max_seq"] == 0
          and st["confirmed"] == 0 and st["refused"] == 0 and st["unparseable"] == [])

    print("== propose: durable pending, prior16 externally verified ==")
    seat = json.loads(seat_line)
    seat["caps"] = int(seat["caps"]) + 1
    mutated = json.dumps(seat)
    rc, out, _ = run("edits", "propose", "--op", "upsert-seat", "--card", mutated,
                     "--home", home)
    check("propose rc0", rc == 0, out)
    pr = jout(out)
    check("propose ok, id e1", pr["ok"] is True and pr["proposal_id"] == "e1", out)
    check("propose echoes op + actor", pr["op"] == "upsert-seat" and pr["actor"] == "terminal")
    check("stream untouched by propose", len(stream_lines(stream)) == 3)
    rc, out, _ = run("edits", "status", "--home", home)
    st = jout(out)
    check("status pending=1 exists=true", st["exists"] is True and len(st["pending"]) == 1)
    p0 = st["pending"][0]
    check("pending prior16 == external sha256[..16]", p0["prior16"] == prior16(stream),
          f'{p0["prior16"]} vs {prior16(stream)}')
    check("pending card is the mutated card", json.loads(p0["card"]) == seat)
    check("pending state word", p0["state"] == "pending" and p0["resolved_by"] is None)

    print("== confirm: gate closed rc1, nothing written ==")
    no_ledger = os.path.join(sbx, "absent.ledger")
    rc, out, _ = run("edits", "confirm", "--id", "e1", "--actor", "op",
                     "--warden", no_ledger, "--home", home)
    cf = jout(out)
    check("confirm rc1 on absent ledger", rc == 1, f"rc={rc} {out}")
    check("gate-closed message", "gate closed" in cf["error"], cf["error"])
    check("stream still 3 lines", len(stream_lines(stream)) == 3)
    check("journal still 1 row", len(stream_lines(journal)) == 1)

    print("== confirm: through a real card.open row ==")
    ledger = os.path.join(sbx, "warden.jsonl")
    with open(ledger, "w", encoding="utf-8", newline="\n") as f:
        f.write(warden_open_row("op", "CARD-1") + "\n")
    rc, out, _ = run("edits", "confirm", "--id", "e1", "--actor", "op",
                     "--warden", ledger, "--home", home)
    cf = jout(out)
    check("confirm rc0", rc == 0, out)
    check("confirm ok + warden card", cf["ok"] is True and cf["warden_card"] == "CARD-1", out)
    check("applied_key names the seat", cf["applied_key"].startswith("seat/"), cf["applied_key"])
    lines = stream_lines(stream)
    check("stream grew to 4", len(lines) == 4)
    check("appended card == mutated card", json.loads(lines[-1]) == seat)
    view = json.load(open(os.path.join(home, "seats-view.json"), encoding="utf-8"))
    seats = view.get("seats") or view.get("rows") or []
    if isinstance(seats, list) and seats and isinstance(seats[0], dict):
        got = next((s for s in seats if s.get("id") == seat["id"]), None)
        check("view resynced to the new caps", got is not None and int(got["caps"]) == int(seat["caps"]))
    else:
        check("view resynced (view parsed, seats not a list — inspect)", False, type(seats))

    print("== no-op + stale + refuse refusals ==")
    rc, out, _ = run("edits", "propose", "--op", "upsert-seat", "--card", mutated,
                     "--home", home)
    check("re-propose rc1 no-op", rc == 1 and "no-op" in jout(out)["error"], out)

    # Proposal ids ARE journal seqs (confirm/refuse consume seqs too) —
    # never hardcode them; read each id from its own propose response.
    seat2 = json.loads(seat_line)
    seat2["caps"] = int(seat2["caps"]) + 2
    rc, out, _ = run("edits", "propose", "--op", "upsert-seat",
                     "--card", json.dumps(seat2), "--home", home)
    pid2 = jout(out)["proposal_id"]
    check("propose seat2 rc0", rc == 0 and pid2 != "e1", out)
    with open(stream, "a", encoding="utf-8", newline="\n") as f:
        f.write(extra_line + "\n")
    rc, out, _ = run("edits", "confirm", "--id", pid2, "--actor", "op",
                     "--warden", ledger, "--home", home)
    check("confirm rc1 stale after stream moved", rc == 1 and "stale" in jout(out)["error"], out)

    seat3 = json.loads(seat_line)
    seat3["caps"] = int(seat3["caps"]) + 3
    rc, out, _ = run("edits", "propose", "--op", "upsert-seat",
                     "--card", json.dumps(seat3), "--home", home)
    pid3 = jout(out)["proposal_id"]
    check("propose seat3 rc0", rc == 0, out)
    rc, out, _ = run("edits", "refuse", "--id", pid3, "--actor", "op", "--home", home)
    rf = jout(out)
    check("refuse rc0 ok", rc == 0 and rf["ok"] is True, out)
    rc, out, _ = run("edits", "status", "--home", home)
    st = jout(out)
    # A STALE confirm refuses to apply but does NOT resolve the proposal —
    # only an explicit refuse/confirm does. The stale pid2 stays pending.
    check("status refused=1, stale proposal still pending",
          st["refused"] == 1 and [p["id"] for p in st["pending"]] == [pid2], out)
    rc, out, _ = run("edits", "confirm", "--id", pid3, "--actor", "op",
                     "--warden", ledger, "--home", home)
    check("confirm rc1 NotPending after refuse", rc == 1 and "not pending" in jout(out)["error"].lower(), out)
    rc, out, _ = run("edits", "confirm", "--id", "e99", "--actor", "op",
                     "--warden", ledger, "--home", home)
    check("confirm rc1 unknown id", rc == 1 and "unknown proposal" in jout(out)["error"], out)

    print("== usage / defect exits (rc2) ==")
    rc, _, _ = run("edits", "status", "--bogus", "--home", home)
    check("unknown flag rc2", rc == 2)
    rc, _, _ = run("edits", "propose", "--home", home)
    check("propose without --op/--card rc2", rc == 2)
    rc, _, _ = run("edits", "propose", "--op", "upsert-seat",
                   "--card", real[0] + "\n" + seat_line, "--home", home)
    check("two-card --card rc2", rc == 2)
    prov = next((l for l in real if "lane_type" in card_keys(l) and "provider" not in card_keys(l)), None)
    if prov:
        rc, _, _ = run("edits", "propose", "--op", "upsert-seat", "--card", prov, "--home", home)
        check("op-word x card-class mismatch rc2", rc == 2)
    rc, _, _ = run("edits", "bogus-verb", "--home", home)
    check("unknown verb rc2", rc == 2)

    print("== REAL home read-only smoke ==")
    rc, out, _ = run("edits", "status")
    st = jout(out)
    check("real home status rc0 exists=false", rc == 0 and st["exists"] is False, out)
    check("real home census empty", st["pending"] == [] and st["max_seq"] == 0)

    print(f"\n{PASS}/{PASS + FAIL}" + (" PASS" if FAIL == 0 else f" ({FAIL} FAIL)"))
    return 1 if FAIL else 0


if __name__ == "__main__":
    sys.exit(main())
