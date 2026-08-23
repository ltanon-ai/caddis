# CAL-PLAN answer key — the oracle

Never shown to the model under calibration. The pack's receipts are
fixtures, so calibration closes without a strong lane: the KEY is the
intent oracle, `validate --plan` is the structure oracle.

- CAL-PLAN-review-a: **rejected** — CARD-A edits `l1_a.py`/GREETING,
  which serves test_l1, not this goal; and NO child implements `clamp`,
  so test_l2 stays red even if CARD-B lands. Structure is valid (that is
  the trap): wrongness lives only in intent.
- CAL-PLAN-review-b: **accepted** — `clamp` (l2_a.py) and `is_even`
  (l2_b.py) exactly cover test_l2's surface, one child per symbol,
  paths disjoint, receipt present.
