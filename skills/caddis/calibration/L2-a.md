---
id: CAL-L2-a
class: calibration
owner: ladder
---

# CAL-L2-a: implement one small function (L2)

## Done-When

- fixtures/test_all.py::clamp-related assertions pass

## RED-TEST

- the clamp assertions fail before the edit

## EXECUTION

```yaml
level: L2
blast: 1
claims-forbidden: true
anchors:
  - path: fixtures/l2_a.py
    content: |
      def clamp(v, lo, hi):
          # TODO implement
          return v
allowlist:
  - edit fixtures/l2_a.py
```
