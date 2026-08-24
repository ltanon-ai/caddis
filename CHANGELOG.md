# Changelog

Rendered from the work history. The engine's development log — 25 red-first
hardening iterations, each with its failing test, measurement and review — lives
in the private workshop; what ships here is the product view.

## 0.2.3 — measurement, destructive laws, and the rlm nerve

The ledger learns to answer questions about itself, the law engine grows
the destructive-command class, and a third harness family gets its nerve.

- **Every dispatch now carries its measured context.** Ladder profiles
  (schema v3, tolerant of every older row) stamp each dispatch with
  `context_bytes` — the measured utf-8 byte sum of card + anchors + annex
  at dispatch time — and the goal tree's `LeafDispatch` event carries the
  same number, with old logs parsing unchanged. Context rot becomes a
  number you can plot, not a feeling.
- **`caddis-warden report` reads the ledger back.** Counts by verdict and
  caller, first/last timestamps, deny reasons grouped by the law id the
  why field carries; `--from`/`--since`/`--verdict`/`--last` narrow it,
  `--json` feeds machines. The nightly question now has a digest command.
- **Replay counts what fired.** `--replay` gains a per-law summary —
  deny and steer fires over the judged rows, plus the never-fired list
  from the registry — so coverage is read, not assumed.
- **The destructive-command laws landed** (council-shaped): `rm -rf`
  against absolute roots (`/`, `/usr`, `C:\`, `%USERPROFILE%`, `$HOME`…),
  `..` escapes above the workspace, and NULL variable expansions
  (`rm -rf $UNSET_VAR/` is effectively `rm -rf /`) are DENIED in every
  flag spelling and through `sudo`/`env` wrapper descent; named relative
  subpaths (`build/`, `dist/`) stay free — build dirs are legitimate
  work; bare `*` denies at the workspace root and steers after a `cd`;
  `curl | bash` STEERS with a domain trustlist (rustup, Homebrew, bun
  named; untrusted domains show the exact URL) and the class is never
  hard-denied — installer false positives are what get a warden
  switched off.
- **The rlm adapter.** Kernel-based harnesses get a nerve too:
  `adapters/rlm/warden_repl.py` wraps the standard-library exec surface
  (`subprocess.*`, `os.system`, `os.popen`) — never harness internals —
  denying before the exec with the warden's reason, steering beside the
  output, stamped `CADDIS_WARDEN_FROM=rlm`. The boundary is stated where
  a reader meets it: pure-Python destructiveness without a shell call is
  out of scope, the same register as THREAT-MODEL's embedded-program
  boundary.

## 0.2.4 — onboarding stops lying about the skill

- **Re-running `onboard` updates the skill again.** `cp -r SRC DEST` copies
  INTO an existing DEST, so the second onboarding — the one ONBOARD.md tells
  you to run after every warden update — nested the fresh skill at
  `DEST/caddis/` and left the stale copy upstairs, while printing
  "agent helper installed" and exiting 0. Measured on a real machine: the
  loaded `SKILL.md` stayed a day old and `ladder.py` was 4709 bytes against
  the 9360 that had just been installed one level down. The install now lives
  in `tools/install-skill.sh`, replaces the destination instead of copying
  into it, refuses a directory it cannot prove is a caddis skill, warns on
  stderr instead of failing silently, and leaves `__pycache__` behind.
- **The re-run is now tested.** `tools/test_install_skill.py` runs the second
  install — the case the self-proof can never reach, because it only ever runs
  on a fresh machine — across the whole CI matrix.

## 0.2.2 — the latch fix

- **`shell.exit-code-through-pipe` no longer inherits.** The check tracked
  "a pipeline appeared somewhere earlier on this line" rather than "the
  command immediately before this `$?` was a pipeline", so
  `a | b; c > out 2>&1; rc=$?` was flagged — the exact shape the finding's
  own message prescribes as the remedy. A check that fires on the correct
  idiom trains its reader to skip the channel, which is the single failure
  this crate keeps warning about. Found by dogfooding: it fired on a session
  that was already capturing the status directly.
- **The warden crate is projected, not hand-synced.** It was byte-identical
  in both repos and kept so by hand — parallel maintenance the twin-repo
  doctrine forbids. It is in the publish manifest now, so drift is
  impossible by construction.

## 0.2.1 — the work-discipline release

The warden engine is unchanged; everything around it grew a second half.
Caddis is now a conscience **and a work discipline**: judging calls, recording
verdicts, and governing the work itself.

- **Cards grew a strict contract** — `validate --strict` now demands every
  field the dispatch machinery reasons about: `level` (L1–L3), `blast`
  (1..=3 paths), `claims-forbidden`, verbatim `anchors`, an exact
  `allowlist`. CONTINUATION and SPLIT annexes ride without ever broadening
  the card. Plan cards (`class: plan`) carry CHILDREN + REVIEW and their own
  oracle (`--plan`) — decomposition is not execution and is not checked as
  execution. `##` sections accepted additively; fenced blocks are always
  content.
- **The ladder** — measured L1–L3 levels for local-model executors.
  Calibration packs ship (a line replace, a function stub, a two-file
  change, plan-review exercises) with the KEY staying operator-side.
  Transforms are hypotheses with retirement (3 uses, 0 conversions → never
  proposed again); preset switching is hysteresis N=4; every dispatch stamps
  a strategy row into an executor profile that is telemetry, never memory.
- **The goal tree** (`crates/caddis-tree`) — a plan decomposes into ordered
  child cards; a walker dispatches leaves through a named executor trait.
  Append-only event log, one writer, global attempt/cost caps checked in
  two phases, kill-mid-tree resume from the file alone, AlreadyDone
  refusals, staged failure bubbling with one replan. The acceptance bench
  gates leaves with the target repository's own checker run as a real
  subprocess — no simulated verdicts.
- **The skill** — `skills/caddis/` is the agent-side manual the harness
  loads: card-first as the default, the conscience commands, the ladder's
  mechanical rules.
- **Adapters** — `lanes.json` cwd-prefix → label stamping (longest prefix
  wins, filesystem-aware case folding) and `CADDIS_WARDEN_STAND_ASIDE=1`
  for fleets wiring only some directories.
- **Quality** — coverage pipeline feeding SonarQube real numbers
  (new-code coverage 91.4% at landing), CI matrix on Linux, Windows and
  macOS, doc-reality audits keeping the README honest against the code.

Proven live end-to-end: the acceptance bench ran two real goals through a
real checker — one first-attempt green, one bubble-up; the coverage gate
passed on the pushed head.

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
