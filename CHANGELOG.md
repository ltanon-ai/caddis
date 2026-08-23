# Changelog

Rendered from the work history. The engine's development log — 25 red-first
hardening iterations, each with its failing test, measurement and review — lives
in the private workshop; what ships here is the product view.

## 0.2.0 — the trust surface release

The engine is unchanged; the trust around it is now explicit.

- **THREAT-MODEL.md** — what caddis protects, its three trust assumptions,
  in-scope and out-of-scope attack classes, and the deliberate
  fail-open/fail-closed trade-off. README claims made precise: every tool
  call *the harness routes through the installed adapter*; attachment is
  broad, not universal.
- **Ledger semantics** (PROTOCOL) — append-only is program behavior, not
  protection; what a sequence gap proves and what it never will; the
  ledger is observability, not security evidence.
- **DISTRIBUTION.md** — runtime vs build requirements split; verify what
  you install; update with rollback and re-proof; removal; platform
  honesty (Windows end-to-end, Linux compiles in CI, macOS untested).
  SHA256SUMS tooling ships (tools/checksums.sh); this release publishes
  the Windows x64 binary with its checksum — other platforms build from
  source.
- **SUPPORT.md** — the adapter matrix: two adapters *supported* with live
  proofs and repro commands, generic hook/RPC marked protocol-compatible
  and honestly untested.
- **Public proofs** — CI badge on the README, a one-command self-count of
  the suite, and a fully documented red-first law cycle in LAWS.md.
- **Projection mechanism** — every public file now has one canonical home
  in the private workshop; the public repo is a verified projection.

## 0.1.0 — the conscience, first public cut

- **caddis-warden**: the law engine. Deny/steer/allow at the tool boundary; real
  shell grammar (wrapper descent with per-runner flag tables, command positions,
  comments, quoting, escaped separators, getopt abbreviation rules);
  sensitive-path first-class law; suppression-marker law with prose exemption
  and line-scoped escape hatch; secret-literal detection; append-only verdict
  ledger with idempotency keys and per-harness `from:` attribution
  (`CADDIS_WARDEN_FROM`); fail-closed on unreadable verdicts, allow-loud when
  unspawnable. 170+ tests. - **caddis-core**: the kernel — envelope → policy →
  idempotency → ledger. Zero dependencies. - **caddis-card**: the work-unit law
  — card schema enforcing Done-When + RED-TEST (falsifiable completion + proof
  the card is not lying). - **adapters/caddis-warden.ts**: the nerve —
  CALLER-stampable per agent, no policy, standard extension API shape. -
  **onboard**: self-proving install — the script fails unless it can show you a
  denial, an allow, and the ledger row that recorded both. - **PROTOCOL.md /
  LAWS.md / ONBOARD.md**: the wire contract, the law registry, and the
  onboarding story.

Proven live end-to-end: a fresh clone from the public repo built in ~2 s and
denied a force-push with a fresh ledger row.
