---
id: CAL-PLAN-review-b
class: calibration
owner: ladder
---
# CAL-PLAN-review-b: intent review of a path-valid plan (expected: ACCEPT)

# Done-When
- reviewer output is two parseable lines (`verdict:` + `checks:`)
- the verdict matches the KEY for this card

# RED-TEST
- an uncalibrated reviewer may reject a sound plan (over-firing) — the
  KEY disagrees; that mismatch is the calibration signal

# GOAL

fixtures/test_all.py::test_l2 must pass: clamp(15, 0, 10) == 10,
is_even(4) is True, is_even(3) is False. Today both are stubs.

# PLAN

```text
---
id: PLAN-CAL-b
class: plan
owner: ladder
---
# Split test_l2 into two child edits

# Done-When

- fixtures/test_all.py::test_l2 passes

# RED-TEST

- test_l2 fails today: clamp and is_even are stubs

# CHILDREN

- id: CARD-A
  order: 1
  paths: fixtures/l2_a.py
  symbols: clamp
- id: CARD-B
  order: 2
  paths: fixtures/l2_b.py
  symbols: is_even

# REVIEW

reviewer: ladder-fixture
verdict: accepted
checks: clamp and is_even each get exactly one child; paths disjoint
```

# OUTPUT

First line `verdict: accepted` or `verdict: rejected`, second line
`checks: <one sentence>`. Nothing else.
