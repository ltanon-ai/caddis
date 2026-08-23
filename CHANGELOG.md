# Changelog

Rendered from the work history. The engine's development log — 25 red-first
hardening iterations, each with its failing test, measurement and review —
lives in the private workshop; what ships here is the product view.

## 0.1.0 — the conscience, first public cut

- **caddis-warden**: the law engine. Deny/steer/allow at the tool boundary;
  real shell grammar (wrapper descent with per-runner flag tables, command
  positions, comments, quoting, escaped separators, getopt abbreviation
  rules); sensitive-path first-class law; suppression-marker law with
  prose exemption and line-scoped escape hatch; secret-literal detection;
  append-only verdict ledger with idempotency keys and per-harness `from:`
  attribution (`CADDIS_WARDEN_FROM`); fail-closed on unreadable verdicts,
  allow-loud when unspawnable. 170+ tests.
- **caddis-core**: the kernel — envelope → policy → idempotency → ledger.
  Zero dependencies.
- **caddis-card**: the work-unit law — card schema enforcing Done-When +
  RED-TEST (falsifiable completion + proof the card is not lying).
- **adapters/caddis-warden.ts**: the nerve — CALLER-stampable per agent,
  no policy, standard extension API shape.
- **onboard**: self-proving install — the script fails unless it can show you
  a denial, an allow, and the ledger row that recorded both.
- **PROTOCOL.md / LAWS.md / ONBOARD.md**: the wire contract, the law
  registry, and the onboarding story.

Proven live end-to-end: a fresh clone from the public repo built in ~2 s
and denied a force-push with a fresh ledger row.
