---
id: CAL-L1-a
class: calibration
owner: ladder
---

# CAL-L1-a: one verbatim line replace (L1)

# Done-When

- fixtures/test_all.py::test_l1 passes for this file's assertion
- l1_a.py contains the new line exactly

# RED-TEST

- the assertion for l1_a.py fails before the edit

# EXECUTION

level: L1
blast: 1
claims-forbidden: true
anchors:
  - path: fixtures/l1_a.py
    content: |
      GREETING = "todo"
allowlist:
  - edit fixtures/l1_a.py
