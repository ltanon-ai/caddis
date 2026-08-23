"""ladder.py — the executor ladder: profiles, mechanical level rules.

Lives beside the caddis skill because it IS the skill's state machine
(quorum card-ladder ruling: the ladder lives agent-side; no engine
daemon). Profiles are named capability telemetry at
~/.caddis/executor-profiles/<model>.json — never "memory", never merged
into the warden ledger (correction #2).

Rules (quorum-pinned, correction #3/#4 + sol refinements adopted):
- start L1; promotion = 2 consecutive FIRST-ATTEMPT UNTRANSFORMED accepts
- blast violation / claims failure / retired-transform hit → immediate −1,
  floor clamps at L1
- transform with >=3 recorded uses and 0 conversions retires (never
  re-proposed); transforms are hypotheses — record whether the retry
  actually converted
- fallback tax (strong-lane closures) recorded per level: the honest cost
- plan oracle (BC2, card-tree quorum): its own counters — proposed /
  well_formed / intent_accepted / intent_rejected — and its own ladder;
  promotion = 2 consecutive intent_accepted; plan outcomes never touch
  exec telemetry (attribution first)
- schema v2 (BC4): per-dispatch stamped rows {goal_id, card_id, strategy,
  model_fingerprint, blast_set, outcome} — the strategy ledger. NEVER
  extended into the warden envelope. Determinism = same (goal_id,
  strategy) → same blast_set over TAGGED rows only; hysteresis N=4 gates
  any preset switch; presets-only until determinism holds.
"""
import json
import os
import time

LEVELS = ["L1", "L2", "L3"]
HOME = os.path.expanduser("~")
PROFILES = os.path.join(HOME, ".caddis", "executor-profiles")


def blank(model):
    return {
        "model": model,
        "fingerprint": {"noted_at": int(time.time())},
        "version": 2,
        "stamped": [],
        "level": "L1",
        "levels": {
            lv: {"attempts": 0, "accepts": 0, "fallbacks": 0}
            for lv in LEVELS
        },
        "streak": {"clean_first_attempts": 0},
        "plan": {
            "proposed": 0,
            "well_formed": 0,
            "intent_accepted": 0,
            "intent_rejected": 0,
            "level": "L1",
            "streak": 0,
        },
        "transforms": {},
        "history": [],
    }


class Profile:
    def __init__(self, path):
        self.path = path
        self.data = blank("")  # replaced by load(); never None

    def load(self):
        with open(self.path, encoding="utf-8") as f:
            self.data = json.load(f)
        self._upgrade()
        return self

    def _upgrade(self):
        """Schema v2 (BC4): stamped rows; a v1 file gains them empty."""
        self.data.setdefault("version", 2)
        self.data.setdefault("stamped", [])
        self.data.setdefault(
            "plan",
            {"proposed": 0, "well_formed": 0, "intent_accepted": 0,
             "intent_rejected": 0, "level": "L1", "streak": 0},
        )

    def save(self):
        os.makedirs(os.path.dirname(self.path), exist_ok=True)
        with open(self.path, "w", encoding="utf-8", newline="\n") as f:
            json.dump(self.data, f, indent=2)

    # ── level machinery ──────────────────────────────────────────────────
    def level(self):
        return self.data["level"]

    def _demote(self):
        idx = LEVELS.index(self.data["level"])
        self.data["level"] = LEVELS[max(0, idx - 1)]
        self.data["streak"]["clean_first_attempts"] = 0

    def _maybe_promote(self):
        if self.data["streak"]["clean_first_attempts"] >= 2:
            idx = LEVELS.index(self.data["level"])
            if idx + 1 < len(LEVELS):
                self.data["level"] = LEVELS[idx + 1]
            self.data["streak"]["clean_first_attempts"] = 0

    # ── recording ────────────────────────────────────────────────────────
    def record(self, level, outcome, attempt=1, transform=None, mode=None):
        self._tally(level, outcome)
        self._track_streak(level, outcome, attempt, transform, mode)
        if transform:
            self.record_transform(
                transform, converted=(outcome == "accept" and attempt > 1)
            )

    def _tally(self, level, outcome):
        """Exec counters: attempts, and fallbacks as the honest tax."""
        lv = self.data["levels"][level]
        lv["attempts"] += 1
        if outcome == "fallback":
            lv["fallbacks"] += 1
        elif outcome == "accept":
            lv["accepts"] += 1

    def _track_streak(self, level, outcome, attempt, transform, mode):
        """Promotion streak and demotion laws, exactly as quorum-pinned."""
        clean = outcome == "accept" and attempt == 1 and transform is None
        streak = self.data["streak"]
        if clean:
            streak["clean_first_attempts"] += 1
            self._maybe_promote()
        else:
            streak["clean_first_attempts"] = 0
        if mode in ("blast-violation", "claims-violation", "retired-transform"):
            self._demote()
        if outcome == "reject" and mode:
            self.data["history"].append(
                {"ts": int(time.time()), "level": level, "mode": mode}
            )

    def record_transform(self, name, converted):
        t = self.data["transforms"].setdefault(name, {"used": 0, "converted": 0})
        t["used"] += 1
        if converted:
            t["converted"] += 1

    def transform_retired(self, name):
        t = self.data["transforms"].get(name)
        return bool(t) and t["used"] >= 3 and t["converted"] == 0

    def tax(self, level):
        return self.data["levels"][level]["fallbacks"]

    # ── plan oracle (BC2, card-tree quorum 2026-08-23) ───────────────────
    def record_plan(self, outcome):
        """Attribute one proposed plan: 'malformed' (validate_plan
        structure failed), 'rejected' (well-formed, intent review refused)
        or 'accepted' (well-formed, intent accepted). The plan ladder is a
        separate oracle — this never touches exec counters."""
        pl = self.data["plan"]
        pl["proposed"] += 1
        if outcome == "accepted":
            pl["well_formed"] += 1
            pl["intent_accepted"] += 1
            pl["streak"] += 1
            if pl["streak"] >= 2:
                idx = LEVELS.index(pl["level"])
                if idx + 1 < len(LEVELS):
                    pl["level"] = LEVELS[idx + 1]
                pl["streak"] = 0
            return
        pl["streak"] = 0
        if outcome == "rejected":
            pl["well_formed"] += 1
            pl["intent_rejected"] += 1

    # ── schema v2: per-dispatch stamped rows (BC4) ──────────────────────
    def record_dispatch(self, goal_id, card_id, strategy, blast_set, outcome):
        """One stamped row per dispatch: the strategy ledger determinism
        and hysteresis read. NEVER extended into the warden envelope."""
        self.data["stamped"].append({
            "goal_id": goal_id,
            "card_id": card_id,
            "strategy": strategy,
            "model_fingerprint": {
                "model": self.data["model"],
                "noted_at": self.data["fingerprint"]["noted_at"],
            },
            "blast_set": sorted(blast_set),
            "outcome": outcome,
        })


def determinism(rows):
    """BC4: same (goal_id, strategy) must yield the same blast_set, over
    TAGGED rows only (empty strategy = untagged, ignored). Returns
    (holds, offenders)."""
    seen = {}
    offenders = []
    for r in rows:
        if not r.get("strategy"):
            continue
        key = (r.get("goal_id", ""), r["strategy"])
        blast = tuple(r.get("blast_set") or [])
        if key in seen and seen[key] != blast and key not in offenders:
            offenders.append(key)
        seen.setdefault(key, blast)
    return (not offenders), offenders


def should_switch(rows, current, candidate, n=4):
    """Hysteresis N=4 (BC4): switch presets only after n consecutive
    non-accept outcomes under `current` — four is a trend, not noise.
    `candidate` names the destination preset; "switching" to the preset
    already in force is a no-op by definition, never a switch."""
    if candidate == current:
        return False
    tail = [r for r in rows if r.get("strategy") == current][-n:]
    return len(tail) == n and all(r.get("outcome") != "accept" for r in tail)


def default_path(model):
    safe = model.replace("/", "_").replace("\\", "_").replace(":", "_")
    return os.path.join(PROFILES, f"{safe}.json")


def main():
    import sys

    if len(sys.argv) >= 3 and sys.argv[1] == "profile":
        p = Profile(default_path(sys.argv[2]))
        p.load()
        print(f"{p.data['model']}: level={p.level()} "
              f"tax={ {lv: p.tax(lv) for lv in LEVELS} }")
        pl = p.data["plan"]
        print(f"  plan: {pl['proposed']} proposed, "
              f"{pl['intent_accepted']} accepted, level={pl['level']}")
        print(f"  stamped: {len(p.data['stamped'])} rows")
        return 0
    print("usage: ladder.py profile <model>")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
