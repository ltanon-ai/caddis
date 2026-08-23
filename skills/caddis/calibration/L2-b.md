---
id: CAL-L2-b
class: calibration
owner: ladder
---

# CAL-L2-b: implement one small function (L2)

# Done-When

- fixtures/test_all.py::is_even-related assertions pass

# RED-TEST

- the is_even assertions fail before the edit

# EXECUTION

level: L2
blast: 1
claims-forbidden: true
anchors:
  - path: fixtures/l2_b.py
    content: |
      def is_even(n):
          # TODO implement
          return True
allowlist:
  - edit fixtures/l2_b.py
