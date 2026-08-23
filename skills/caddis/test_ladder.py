"""test_ladder.py — CARD-LADDER-1: the profile store and its MECHANICAL rules.

Quorum-pinned (card-ladder 2026-08-23): demotion is immediate on blast
violation / claims failure / retired-transform hit (clamp: never below L1);
promotion requires TWO consecutive FIRST-ATTEMPT UNTRANSFORMED accepts
(sol's refinement, adopted); a transform with >=3 recorded uses and zero
conversions retires and is never re-proposed; fallback tax (strong-lane
closures per level) is recorded as the honest cost signal; profiles live
at ~/.caddis/executor-profiles/<model>.json - named capability telemetry,
never "memory", never merged into the warden ledger.

The module is LOADED BY PATH below (never a sys.path mutation): in the
workshop the ladder lives in skills/caddis/ (not a package root); in the
projected public tree it sits beside this file.
"""
import importlib.util
import json
import os
import tempfile

_HERE = os.path.dirname(os.path.abspath(__file__))
_LADDER = next(
    (
        p
        for p in (
            os.path.join(_HERE, "ladder.py"),
            os.path.join(_HERE, "..", "skills", "caddis", "ladder.py"),
        )
        if os.path.isfile(p)
    ),
    None,
)
assert _LADDER is not None, "ladder.py not found beside or ../skills/caddis/"
_SPEC = importlib.util.spec_from_file_location("ladder", _LADDER)
assert _SPEC is not None and _SPEC.loader is not None
ladder = importlib.util.module_from_spec(_SPEC)
_SPEC.loader.exec_module(ladder)


def fresh(tmp, model="m"):
    p = ladder.Profile(os.path.join(tmp, f"{model}.json"))
    p.data = ladder.blank(model)
    return p


def test_new_profile_starts_at_l1():
    with tempfile.TemporaryDirectory() as tmp:
        p = fresh(tmp)
        assert p.level() == "L1", "the ladder defaults LOW"


def test_promotion_needs_two_consecutive_first_attempt_untransformed_accepts():
    with tempfile.TemporaryDirectory() as tmp:
        p = fresh(tmp)
        p.record("L1", "accept", attempt=1, transform=None)
        assert p.level() == "L1", "one accept is not enough"
        p.record("L1", "accept", attempt=1, transform=None)
        assert p.level() == "L2", "two clean first attempts promote"
        p.record("L2", "accept", attempt=2, transform="pin-line")
        assert p.level() == "L2", "transformed accepts never promote"


def test_blast_violation_demotes_and_clamps_at_l1():
    with tempfile.TemporaryDirectory() as tmp:
        p = fresh(tmp)
        p.record("L1", "accept", attempt=1, transform=None)
        p.record("L1", "accept", attempt=1, transform=None)
        assert p.level() == "L2"
        p.record("L2", "reject", attempt=1, transform=None,
                 mode="blast-violation")
        assert p.level() == "L1", "blast violation is immediate demotion"
        p.record("L1", "reject", attempt=1, transform=None,
                 mode="blast-violation")
        assert p.level() == "L1", "the floor clamps at L1"


def test_transform_retires_after_three_uses_zero_conversions():
    with tempfile.TemporaryDirectory() as tmp:
        p = fresh(tmp)
        for _ in range(3):
            p.record_transform("pin-line", converted=False)
        assert p.transform_retired("pin-line")
        assert not p.transform_retired("imperative-step")
        p.record("L1", "reject", attempt=1, transform=None,
                 mode="retired-transform")
        assert p.level() == "L1"


def test_fallback_tax_is_recorded_per_level():
    with tempfile.TemporaryDirectory() as tmp:
        p = fresh(tmp)
        p.record("L2", "fallback", attempt=3, transform=None)
        p.record("L2", "fallback", attempt=3, transform=None)
        assert p.tax("L2") == 2
        assert p.tax("L1") == 0


def test_profile_roundtrips_to_named_file():
    with tempfile.TemporaryDirectory() as tmp:
        p = fresh(tmp, "ollama_gpt-oss-20b")
        p.record("L1", "accept", attempt=1, transform=None)
        p.save()
        q = ladder.Profile(p.path)
        q.load()
        assert q.data["model"] == "ollama_gpt-oss-20b"
        assert q.data["levels"]["L1"]["attempts"] == 1

def test_plan_counters_attribute_exactly_one_outcome_each():
    with tempfile.TemporaryDirectory() as tmp:
        p = fresh(tmp)
        p.record_plan("malformed")
        p.record_plan("rejected")
        p.record_plan("accepted")
        pl = p.data["plan"]
        assert pl["proposed"] == 3
        assert pl["well_formed"] == 2, "malformed never counts well-formed"
        assert pl["intent_accepted"] == 1
        assert pl["intent_rejected"] == 1


def test_plan_promotion_needs_two_consecutive_intent_accepted():
    with tempfile.TemporaryDirectory() as tmp:
        p = fresh(tmp)
        p.record_plan("accepted")
        assert p.data["plan"]["level"] == "L1", "one accepted plan is not enough"
        p.record_plan("accepted")
        assert p.data["plan"]["level"] == "L2", "two consecutive accepted promote"
        p.record_plan("rejected")
        p.record_plan("accepted")
        assert p.data["plan"]["level"] == "L2", "a rejection breaks the streak"


def test_plan_oracle_never_pollutes_exec_telemetry():
    with tempfile.TemporaryDirectory() as tmp:
        p = fresh(tmp)
        p.record_plan("accepted")
        p.record_plan("accepted")
        assert p.data["level"] == "L1", "plan accepts never promote exec level"
        assert p.data["levels"]["L1"]["attempts"] == 0
        assert p.data["streak"]["clean_first_attempts"] == 0


def test_v2_stamped_rows_roundtrip_and_upgrade():
    with tempfile.TemporaryDirectory() as tmp:
        p = fresh(tmp, "ollama_gpt-oss-20b")
        assert p.data["version"] == 2, "schema v2 is explicit, not implied"
        assert p.data["stamped"] == []
        p.record_dispatch(
            goal_id="g1", card_id="CARD-A", strategy="weak-first",
            blast_set=["a.py"], outcome="accept",
        )
        row = p.data["stamped"][-1]
        assert (row["goal_id"], row["card_id"]) == ("g1", "CARD-A")
        assert row["strategy"] == "weak-first"
        assert row["blast_set"] == ["a.py"] and row["outcome"] == "accept"
        assert row["model_fingerprint"]["model"] == "ollama_gpt-oss-20b"
        p.save()
        q = ladder.Profile(p.path)
        q.load()
        assert len(q.data["stamped"]) == 1, "rows roundtrip through the file"
        legacy = ladder.blank("m")
        del legacy["version"], legacy["stamped"]
        v1 = os.path.join(tmp, "v1.json")
        with open(v1, "w", encoding="utf-8") as f:
            json.dump(legacy, f)
        r = ladder.Profile(v1)
        r.load()
        assert r.data["version"] == 2 and r.data["stamped"] == [], "v1 upgrades on load"


def test_determinism_holds_over_tagged_rows_only():
    rows = [
        {"goal_id": "g1", "strategy": "weak-first", "blast_set": ["a.py"]},
        {"goal_id": "g1", "strategy": "weak-first", "blast_set": ["a.py"]},
        {"goal_id": "g2", "strategy": "weak-first", "blast_set": ["b.py"]},
        {"goal_id": "g1", "strategy": "", "blast_set": ["z.py"]},
    ]
    ok, offenders = ladder.determinism(rows)
    assert ok and offenders == [], "other goals and untagged rows never count"
    rows.append({"goal_id": "g1", "strategy": "weak-first", "blast_set": ["c.py"]})
    ok, offenders = ladder.determinism(rows)
    assert not ok and offenders == [("g1", "weak-first")]


def test_preset_switch_needs_four_consecutive_failures():
    rows = []
    for _ in range(3):
        rows.append({"strategy": "weak-first", "outcome": "reject"})
        assert not ladder.should_switch(rows, "weak-first", "strong-first")
    rows.append({"strategy": "weak-first", "outcome": "reject"})
    assert ladder.should_switch(rows, "weak-first", "strong-first"), "four is a trend"
    rows.append({"strategy": "weak-first", "outcome": "accept"})
    rows.append({"strategy": "weak-first", "outcome": "reject"})
    assert not ladder.should_switch(rows, "weak-first", "strong-first"), "an accept resets the window"


def test_switching_to_the_preset_in_force_is_never_a_switch():
    rows = [{"strategy": "weak-first", "outcome": "reject"}] * 4
    assert not ladder.should_switch(rows, "weak-first", "weak-first")
