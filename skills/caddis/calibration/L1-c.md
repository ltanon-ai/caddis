---
id: CAL-L1-c
class: calibration
owner: ladder
---

# CAL-L1-c: one verbatim line replace (L1)

## Done-When

- fixtures/test_all.py::test_l1 passes for this file's assertion
- l1_c.py contains the new line exactly

## RED-TEST

- the assertion for l1_c.py fails before the edit

## EXECUTION

```yaml
level: L1
blast: 1
claims-forbidden: true
anchors:
  - path: fixtures/l1_c.py
    content: |
      NAME = "x"
allowlist:
  - edit fixtures/l1_c.py
```
