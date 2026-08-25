# Changelog

Rendered from the work history. The engine's development log — 25 red-first
hardening iterations, each with its failing test, measurement and review — lives
in the private workshop; what ships here is the product view.

## 0.3.0 — a conscience that PROVES, not one that comments

0.2.x made caddis a conscience that *comments*. This makes it a system that
*proves*. Five new readers over a ledger that was, until now, write-mostly —
15k rows in and almost nothing out, which is a writer with no reader.

The version is a deliberate exception to this project's patch-only cadence: it
changes what the product is.

### The ledger was corrupt, and everything else stood on it

Found by an adversarial review before any feature was built, then confirmed
against a live 15411-row ledger:

```
20 rows torn (not parseable JSON)
6733 distinct seq values for 15411 rows
8678 rows (56%) carrying a DUPLICATE seq
19 counter resets
```

`writeln!` onto an unbuffered file issues **one write syscall per format
fragment**, and `O_APPEND` is atomic per syscall rather than per call, so two
wardens appending at once interleaved mid-token. A torn row then defeated the
counter recovery in `Ledger::open`, which reset to zero and re-issued the whole
sequence. Reproduced at 286 torn rows out of 320 with eight concurrent writers.

Fixed three ways, each with a test that fails without it: one `write_all` per
row; counter recovery as the maximum over *intact* rows; and the number chosen
under an advisory lock, because an atomic append fixes torn ROWS and does
nothing for duplicate NUMBERS — read-then-write is a race wherever the read sits
outside the exclusion.

**The historical damage is permanent.** An append-only ledger is never rewritten
to hide what happened to it, so a `seq` from before this release does not
identify one row: 8678 of them share a number with something else.

⚠ **The readers below DO cite `seq`** — `receipt` prints `(seq N)` beside each
denial and `propose-laws` stores an `example_seq`. That is exact for rows written
from this release onward, where the counter is chosen under a lock, and only
approximate for the damaged era, where the same number may name several rows.
An earlier draft of this entry claimed every reader cited a physical position
instead. It did not, and a changelog asserting a property the code does not have
is the failure this release is named for. `Ledger::unreadable()` reports the
damaged count, so an EMPTY ledger and a DAMAGED one stop looking alike.

### `caddis receipt` — what one caller actually did

Reconstructs a window from the ledger alone: verdict counts, per-tool counts,
distinct files written with a count each, denials grouped by the law that drew
them *with the rows cited*, laws fired, and cards opened — naming any left
`STILL OPEN`, because work declared and never closed is work whose bounds nobody
confirmed.

It states what it could not see. On a real 24-hour window: 109 rows, of which
**99 commands were withheld** by the masking and size doctrines. Those doctrines
are right; a receipt that hid the consequence would not be.

### `caddis card open|status|close` — a card becomes a fact in the ledger

The card schema has declared `allowlist` and `blast` since the beginning, and
the warden has always seen every write path; nothing connected them. Card state
now lives *in the ledger* — not in a side file that drifts from it, and not in
an environment variable that leaves no trace in an audit.

Four refusals, each an honest floor rather than a restriction: a caller that
names a harness instead of a session cannot hold a card at all; opening twice
refuses instead of nesting; a card whose allowlist names more distinct paths
than its own `blast` allows is refused before any work starts; and `close`
refuses if the card file changed since it was opened, because an executor that
can rewrite its own allowlist mid-work has no card.

`open` prints whether the card BOUNDS anything, because a v1 card declares no
allowlist and there is nothing to enforce. Saying so is the difference between a
mechanism and a reassurance.

### The open card bounds the edits

With a card open, a write outside its declared allowlist is **denied** when the
target was handed over — a file tool's path, or a literal `>` redirect — and
**steered** when it was recovered from command text. Violating your own pinned
declaration is self-contradiction; guessing at a shell command's effects is not.

With no card open, every verdict is byte-for-byte what it was. The gate is
invisible until someone opts in.

**Scope, stated rather than implied:** this is a declaration gate over write
targets the warden is handed, not a filesystem sandbox. Measured on a real
ledger, file-write tools are 13.5% of rows and shell is 75.6%, and a shell
command's write targets are not recoverable in general. Cost: +6.3 ms per
write-tool call.

### `caddis laws` — which rules earn their place

Per law: fires, the deny/steer split, how often a denial was followed by the
same caller getting the same command through, and one of three verdicts —
EARNING, WALLPAPER, DEAD. Every *registered* law appears, including those that
never fired, because a market that lists only what fired cannot report a dead
rule.

First run on a real ledger: **16 laws — 6 DEAD, 1 WALLPAPER, 9 EARNING**, with
one law that had fired twice in its entire life and been routed around both
times.

The worked-around figure is a **heuristic** and is labelled one in the module,
in the report and inside the JSON: the ledger records what a tool was asked to
do, never what happened, so an honest fix-and-retry counts the same as a
circumvention. A lead, never a verdict about an agent.

### `caddis propose-laws` — candidates with their own falsifier attached

Mines the ledger for a signature nobody reads: a command allowed and then
immediately undone. **Every candidate states what it would have cost** — how
many rows across the whole ledger a law on that signature would have denied — so
the false-positive price is visible before adoption rather than after.

On a real ledger this promptly condemned its own output: four candidates, each
seen undone once or twice, each of which would have denied between 87 and 380
real commands. Without that number, a proposal engine would have suggested
banning `python -c`. Nothing here installs a law, and it says so on every run.

### `caddis attest` — proof that travels with the work

For a card that was opened and closed, a proof bundle assembled from data
already recorded: the card and its content hash, the declared allowlist and
blast, the ledger window, every verdict drawn, every distinct file written, and
**the files written outside what the card declared**. `attest --verify` recomputes
every counted claim against the ledger and exits non-zero if any is
contradicted — pinned by tests that tamper with a bundle by hand, because a
verifier nobody has watched go red is not a verifier.

**What it refuses to claim.** The warden fires *before* a tool runs and no row
carries an exit code, so a bundle cannot say a test failed before and passed
after. It says a matching command was ATTEMPTED, and every bundle carries that
limit in a field of its own, so a reader who only ever sees the JSON still sees
what it cannot prove.

### The skill is now keyed to moments, not to commands

Five readers nobody invokes would be the original defect one storey up. The
skill no longer lists commands; it lists the moments — writing a handoff,
finishing a unit, a bee reporting done, reviewing a bundle, hitting a red gate —
and what to run at each, together with what each answer does *not* mean.

### `caddis-organs` — watchdog, canary and checkpoint, present but NOT WIRED

A new crate carrying three self-watching organs ported harness-agnostic from
proven modules: a **watchdog** (probe → restart → backoff → blocker, blockers
persisted as JSONL so they outlive the process that filed them), a **canary**
(an 11-hop golden path over the real substrate — envelope, policy, idempotency,
ledger append and read-back, checkpoint round-trip, watchdog self-probe — where
any RED tells the host to halt and DEGRADED never does), and a **checkpoint**
store (pre-mutation snapshots, with absent files tombstoned so created-file
mutations undo too). Zero dependencies beyond the kernel; sync, std only.

**What it does not do yet: anything, unless you call it.** Nothing in the
shipped `caddis-warden` binary depends on this crate — it is a library with no
caller in-tree, which by this project's own standard is a writer with no reader.
It ships because the code is real, tested and cross-platform, not because it is
part of the loop. Wiring it is a later release, and this note exists so the gap
is stated rather than discovered.

Landed with three defects fixed that its own arrival exposed: it did not compile
on Linux or macOS (`if cfg!(windows)` type-checks both arms, so a Windows-only
import was compiled everywhere — the attribute form removes the arm), a hang
fixture that only made sense on Windows, and, in the split that brought the
files under this project's 280-line law, a public path broken by an alias. The
last of those is now pinned from outside the crate by an integration test,
because in-crate tests import through `use super::*` and cannot see it.

## 0.2.9 — the tools stop lying about themselves

Five defects, all of the same family: a thing that reported success, or
silence, where it had not actually looked.

- **`--version` and `--help` answered with a denial and exit 0.** Every
  invocation fell through to "read a request frame from stdin", so the first
  command a new user types reported that their install had refused them —
  while claiming success. Version and help now print and exit 0, an unknown
  argument exits 2 with usage and emits no verdict, and a release build can
  carry its release name so a downloaded binary can say which one it is. The
  frame path is byte-for-byte unchanged and pinned by a test.
- **`onboard` located itself with `$0`, which is wrong the moment it is
  sourced** — and the CI matrix sources it. Every `$(dirname "$0")` path
  pointed outside the repository there, so the skill install had been failing
  in CI silently. It resolves through `${BASH_SOURCE[0]}` now, and a test
  sources it the way the matrix does.
- **`--replay` hid what it could not measure.** Soft-finding drift was folded
  into "unchanged", and a bare skip count made a two-thirds-unreadable ledger
  look like a clean one. The report now names NOW-STEERS / NO-LONGER-STEERS
  and states its own coverage with a reason for every skipped row. On a real
  14036-row ledger: 33.2% re-judged, 15 commands that had stopped steering
  finally visible.
- **The default branch did not pass its own `lint:rust`.** `cargo fmt --all
  --check` was red on main. Formatting also pushed `checks/rmrf.rs` past the
  280-line cap, so the destructive law is now split into the law itself and
  `rmrf_operand.rs`, with the nested flag scan extracted — behaviour
  unchanged, all seven rmrf tests still passing.
- **The Sonar gate was ERROR.** A blocker (an adapter `main()` returning a
  constant int on a contract that decides through stdout), a critical
  (cognitive complexity 24 in the incident-log parser, now a loop that skips
  quoted runs whole instead of carrying an `in_string` state machine), and
  four smaller findings. The calibration fixtures leave issue scope rather
  than being "fixed": they are deliberately unimplemented stubs and
  implementing them would delete the exercise.

## 0.2.8 — the banner catches up with the logo

The 0.2.7 logo never reached the public tree (generated image files are not
projected — a copy step the release missed), and the README's top image
is the banner, so nothing visibly changed. This release lands both: the
new logo in `assets/logo.png` and a banner regenerated in the same style
— the pebble spiral flowing left, water-light lines opening the dark
right for the wordmark.

## 0.2.7 — the logo finally is a caddis

The old mark was abstract geometry — teal fins around an orange circle —
with no connection to the caddisfly story the project is named for. The
new logo, generated with Higgsfield's Nano Banana Pro and picked by the
operator from six candidates, is the larva's case itself: a spiral of
glossy teal pebbles around a glowing orange heart on a dark riverbed —
alive, legible at favicon size, and unmistakably the thing the name says.


## 0.2.6 — changelog block order fix

The 0.2.4 block sat below 0.2.3 where its author left it; descending
version order restored (pure move, no content change).

## 0.2.5 — the description catches up with the shipped code

A docs-only release: the code shipped in 0.2.3, the words catch up now.
The engine is unchanged.

- **LAWS.md shows the whole registry.** The HARD table grows from five
  rows to the full eleven (the destructive-command class, the
  rewritten-repo latch, the hook/signing/skip-ci/osv laws beside the
  originals), and the SOFT section gains its first table — nine steers,
  one line each, severities taken from the one registry table in code.
- **README answers 0.2.3's questions.** The glance table gains the Report
  row; the destructive-command class gets its own bullet; the ledger
  section names `report` with its full flag set; replay documents the
  law-fires summary and the never-fired list; the adapters section is
  three shapes — extension, hook, kernel; the ladder row is schema v3
  with `context_bytes` and why; every number is re-measured (6 448
  source lines, 253 Rust tests, 43 in the CI matrix) and the
  requirements cover the rlm nerve (Python 3.8+).
- **MEMORY.md leads with the reading end.** `caddis-warden report`
  examples first; raw grep is demoted to the power-user fallback it
  always was.
- **SUPPORT.md ships three supported rows** — the rlm nerve with its own
  repro (a `WardenRefusal` naming the law, the call never runs) — and the
  version note records all three live proofs.
- **The diagrams caught up too.** The architecture and verdict diagrams
  draw three adapter shapes (extension `.ts`, hook `.py`, kernel `.py`),
  regenerated from the shipped renderer and visually verified.

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
