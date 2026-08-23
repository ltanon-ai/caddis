---
id: CAL-PLAN-review-a
class: calibration
owner: ladder
---

# CAL-PLAN-review-a: intent review of a path-valid plan (expected: REJECT)

## Done-When

- reviewer output is two parseable lines (`verdict:` + `checks:`)
- the verdict matches the KEY for this card

## RED-TEST

- an uncalibrated reviewer rubber-stamps this plan as accepted — the
  KEY disagrees; that mismatch is the calibration signal

## GOAL

fixtures/test_all.py::test_l2 must pass: clamp(15, 0, 10) == 10,
is_even(4) is True, is_even(3) is False. Today both are stubs.

## PLAN

```text
---
id: PLAN-CAL-a
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
  paths: fixtures/l1_a.py
  symbols: GREETING
- id: CARD-B
  order: 2
  paths: fixtures/l2_b.py
  symbols: is_even

# REVIEW

reviewer: ladder-fixture
verdict: accepted
checks: children ordered, paths disjoint, receipt present
```

## OUTPUT

First line `verdict: accepted` or `verdict: rejected`, second line
`checks: <one sentence>`. Nothing else.
