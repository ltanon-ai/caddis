---
id: CAL-L1-b
class: calibration
owner: ladder
---

# CAL-L1-b: one verbatim line replace (L1)

## Done-When

- fixtures/test_all.py::test_l1 passes for this file's assertion
- l1_b.py contains the new line exactly

## RED-TEST

- the assertion for l1_b.py fails before the edit

## EXECUTION

```yaml
level: L1
blast: 1
claims-forbidden: true
anchors:
  - path: fixtures/l1_b.py
    content: |
      LIMIT = 0
allowlist:
  - edit fixtures/l1_b.py
```
