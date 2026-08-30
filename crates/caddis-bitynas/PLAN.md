# PLAN — CARD-BITYNAS-1 (unit H1: lease-core crate `caddis-bitynas`)

## CARD

A coder bee landing this card gives caddis a lease core: open the GPU-pool journal
`pool/leases.jsonl`, atomically claim / heartbeat / release named slots with TTL-based
stale preemption that is NEVER silent (`PeremptionEvent` on every reclaim), plus a local
`lane_allowed` guard refusing `droid` tiers (O2) — proven by ≥6 green tests + a doctest,
committed locally on branch `bitynas/h-1-lease`.

## ANSWERS

(Interview self-answered as the operator's proxy. Facts were looked up in-repo, not asked.)

1. **Atomicity scope** — single-writer process: `&mut self` serializes every check-then-act;
   the journal buys crash-recovery, not cross-process locking. Why: caddis-core's
   `ledger_lock` is PRIVATE (`mod ledger_lock;` in its lib.rs) so it cannot be imported, and
   replicating 8.4 KB of hard-won lock law exceeds the card. The bitynas daemon organ (later
   unit) is the single writer. Multi-writer use is a documented limitation (see RISKS).
2. **Journal encoding** — hand-rolled flat op-tagged JSONL rows (`claim`/`heartbeat`/
   `release`/`reclaim`), last-row-wins fold. Why: house law is std-only beyond serde
   (`serde_json` is dev-only everywhere here; deliberate's organs law bans registry deps
   outright), and `caddis-core/src/ledger_row.rs` is the precedent for hand-rolled rows.
   Replicate its `esc()` law verbatim (a raw `\n` ENDS a JSONL record — CARD-WARDEN-1 lesson).
3. **Clock** — real `SystemTime`, formatted RFC3339-UTC `"YYYY-MM-DDTHH:MM:SSZ"` by
   replicating the Hinnant civil-day math from `caddis-organs/src/util_time.rs`
   (`iso8601_from_unix` / `unix_from_iso8601` / `civil_from_days` / `days_from_civil`) into a
   private `src/rfc3339.rs`. Why: caddis-organs is not an allowed dep; the algorithm is
   ~60 lines, public domain, and already proven in this repo.
4. **`is_stale` semantics** — exactly the card's two args:
   `is_stale(&self, now_utc: &str, now_hb: &str) -> bool` = `unix(now_utc) - unix(now_hb) >
   self.ttl_s` (strict `>`). Any parse failure → `false` (fail-closed: a corrupt timestamp
   must never cause a WRONGFUL preemption — the cost of a wrong reclaim is double GPU use).
5. **TTL source** — `pub const DEFAULT_TTL_S: u64 = 900` (15 min), stamped by `claim`.
   Why: the card fixes `claim(slot_id, lane, owner)` with no ttl param; configurability
   belongs to the daemon unit, not the core. CUT: per-claim TTL, `open_with_ttl`.
6. **Error / idempotency law** — `release` on an absent slot → `Ok(())` (idempotent, POSIX
   unlink semantics); `heartbeat` on an absent slot → `Ok(None)` (caller sees the lease is
   gone); wrong owner anywhere → `Err(BusyError { holder })` (surfaces preemption);
   same-owner re-`claim` → `Err(Busy)` too (one lease per claim; refresh via `heartbeat`).
7. **Event delivery, single-path** — claim-reclaim enqueues into pending, drained by
   `events()`; `sweep` RETURNS its events directly and does NOT also enqueue (no double
   delivery). Journal replay on reopen never re-enqueues historical reclaims (they happened
   pre-crash; re-emitting would double-report). Perėmimas NIEKAD tylaus — but never twice.
8. **`lane_allowed` vocabulary** — closed set `{local, free, mid, premium}` with router
   parity: `trim().to_ascii_lowercase()` then match; `"droid"` refused with the O2 message,
   any other unknown (`"banana"`, `""`) refused with a generic unknown-tier message. Why:
   the card says replicate `caddis-router/src/lane.rs LaneTier::parse` — which refuses ALL
   unknowns, not just droid. Do NOT couple it into `LeaseStore::claim` (the store is
   lane-agnostic data; the guard is the router call-site's duty). Lives in private
   `src/lane.rs` + root re-export; root also re-exports the five contract items.
9. **Test determinism** — no sleeps, no clock injection: tests that need an OLD lease seed
   the journal with a hand-written row (hb `2020-01-01T00:00:00Z`), then `open()` rebuilds
   the index around it. This also proves the row-format contract from outside the crate.
10. **`PeremptionEvent` shape** (card leaves it open) —
    `{slot_id, lane, previous: LeaseRecord, new_owner: Option<LeaseOwner>, at_utc, cause}`
    with `cause = "ttl_expired"`; `new_owner` is `Some(claimer)` on claim-reclaim, `None` on
    sweep. Derives mirror `LeaseRecord` (incl. serde) so the daemon can log it downstream.

OPEN items: none.

Coordination (settled with H2 registry grill, 2026-08-30): PLAN.md paths are disjoint
(theirs `E:\ClaudeToolbox\bee\PLAN.md`); `slot_id` is opaque non-empty string — their
`droid-glm-5-2` slug convention works; `droid` is refused only as a LANE TIER, never inside
a slot_id.

## PLAN

Branch first: `git -C E:\ClaudeToolbox\caddis checkout -b bitynas/h-1-lease` (from `main`).
Every file ≤ 280 lines (split, never trim — house law). All paths new except root Cargo.toml.

1. **`Cargo.toml` (root — the ONE shared file)** — append `"crates/caddis-bitynas",` to the
   `members` list (lines 3–14) and nothing else. RE-READ the file immediately before editing:
   the H2 sibling may have appended its own member concurrently. Edit is append-only so the
   merge is trivial.
2. **`crates/caddis-bitynas/Cargo.toml`** — `[package] name = "caddis-bitynas"`,
   `version = "0.1.0"`, `edition = "2021"`, description citing `CARD-BITYNAS-1` + origin
   (house style: see `caddis-router/Cargo.toml` header comment). `[dependencies]`
   `serde = { version = "1", features = ["derive"] }` ONLY (serde 1.0.229 already in
   Cargo.lock). NO `serde_json`, NO `caddis-core` (its lock is private; time is replicated).
   NEVER `npx`; cargo directly.
3. **`src/rfc3339.rs`** (private `mod rfc3339`) — replicate from
   `caddis-organs/src/util_time.rs`: `now_unix() -> i64`,
   `from_unix(secs: i64) -> String` (`{:04}-{:02}-{:02}T{:02}:{:02}:{:02}Z`, div_euclid /
   rem_euclid so pre-epoch stays total), `unix_from(s: &str) -> Option<i64>` (≥19 chars,
   `'T'` at byte 10, month 1–12 / day 1–31 checked, suffix after seconds ignored),
   + private `civil_from_days` / `days_from_civil` (Hinnant, public domain — say so in a
   comment, like the source file does). Unit tests here: epoch 0 → `1970-01-01T00:00:00Z`;
   `1_700_000_000` → `2023-11-14T22:13:20Z`; `from_unix(unix_from(x)) == x` round-trip;
   garbage (`""`, `"2020-01-01"`, short) → `None`.
4. **`src/lease.rs`** (`pub mod lease`) — the data types:
   - `pub const DEFAULT_TTL_S: u64 = 900;`
   - `pub struct LeaseOwner { pub session_id: String, pub host: String, pub pid: u32 }`
     — derives `Debug, Clone, PartialEq, Eq, Serialize, Deserialize`.
   - `pub struct LeaseRecord` with EXACTLY the card's ten fields: `slot_id: String,
     lane: String, session_id: String, host: String, pid: u32, repo: Option<String>,
     card: Option<String>, taken_at_utc: String, ttl_s: u64, heartbeat_at_utc: String`
     — derives `Debug, Clone, PartialEq, Serialize, Deserialize`.
     `pub fn owner(&self) -> LeaseOwner` (identity triple extractor).
     `pub fn is_stale(&self, now_utc: &str, now_hb: &str) -> bool` per ANSWER 4.
   - `pub struct BusyError { pub holder: LeaseRecord }` — derives `Debug, Clone,
     PartialEq`; `impl Display + std::error::Error`:
     `slot '{slot_id}' busy: held by session '{session_id}' on {host} pid {pid} (lane '{lane}', heartbeat {heartbeat_at_utc})`.
   - `pub struct PeremptionEvent` per ANSWER 10.
   - `claim` leaves `repo`/`card` = `None` (no setters in this unit — registry bee's later
     concern; fields exist for the journal/wire contract).
5. **`src/journal.rs`** (private `mod journal`) — the BYTES, split under the 280 law:
   - `esc(s: &str) -> String` replicated from `caddis-core/src/ledger_row.rs`: escape
     `"` `\` `\n` `\r` `\t` `\b` `\f`, every other C0 as `\u00XX`; nothing above U+001F.
   - Row encoders (flat, exact-field, one line each, `\n`-terminated, all strings `esc`'d):
     `{"op":"claim","slot_id":..,"lane":..,"session_id":..,"host":..,"pid":N,"taken_at_utc":..,"ttl_s":N,"heartbeat_at_utc":..}`
     (`"repo":".."/"card":".."` written only when `Some`; absent = `None` on parse);
     `{"op":"heartbeat","slot_id":..,"at_utc":..}`;
     `{"op":"release","slot_id":..,"at_utc":..}`;
     `{"op":"reclaim","slot_id":..,"at_utc":..,"cause":"ttl_expired","new_owner":null|{session_id,host,pid},"previous":{full record}}`.
   - Strict parser: a line must carry `"op"` and exactly the fields its op requires;
     anything else (torn, unknown op, missing field) counts as unreadable and is SKIPPED,
     never a panic (mirror `Ledger::open` counting law).
   - Fold `Iterator<Row>` → `BTreeMap<String, LeaseRecord>`: claim → insert/replace;
     heartbeat → patch `heartbeat_at_utc`; release → remove; reclaim → remove (the new
     holder, if any, is installed by the claim row written right AFTER the reclaim row).
   - Unit tests: encode→parse round-trip per op; control chars survive `esc`; a torn row
     and an unknown-op row are skipped+counted, not fatal.
6. **`src/store.rs`** (`pub mod store`) — `pub struct LeaseStore` holding `path: PathBuf`,
   `index: BTreeMap<String, LeaseRecord>`, `pending: Vec<PeremptionEvent>`,
   `unreadable: usize`. Doc-comment the SINGLE-WRITER contract on the type (ANSWER 1).
   - `pub fn open(path: &Path) -> std::io::Result<Self>` — `create_dir_all` parent; if the
     file exists, read + parse + fold rows in order (torn rows counted); file itself is
     created lazily on first append (ledger precedent). `pub fn unreadable(&self) -> usize`.
   - Every mutation: decide on the in-memory index → append row(s) → return. Appends use
     `OpenOptions::new().create(true).append(true)`, ONE `write_all` of the complete
     `\n`-terminated row per op (Windows `FILE_APPEND_DATA` is atomic per syscall — the
     CARD-0108 law; NEVER `writeln!` onto the file). Rows are small by construction
     (~≤512 B). No fsync (house ledger appends don't fsync either; "crash" here = process
     death, not power loss).
   - `pub fn claim(&mut self, slot_id: &str, lane: &str, owner: LeaseOwner) ->
     Result<LeaseRecord, BusyError>` — requires non-empty `slot_id` and `lane`, enforced
     with `assert!` plus a `# Panics` doc note: an empty id is a caller BUG (programming
     error), not a busy slot — `BusyError` cannot express it (no holder), and silently
     leasing a `""` key is worse. Tests never pass empty.
     Slot absent → new record (`taken_at_utc == heartbeat_at_utc == now`, `ttl_s =
     DEFAULT_TTL_S`) → journal claim → `Ok`. Held and `is_stale(now, hb)` → journal
     `reclaim` (new_owner = Some(claimer)) THEN journal claim → `pending.push(event)` →
     `Ok(new record)`. Held and fresh → `Err(BusyError { holder: record.clone() })`.
   - `pub fn release(&mut self, slot_id: &str, owner: &LeaseOwner) -> Result<(), BusyError>`
     — absent → `Ok(())`; mismatch → `Err(Busy{holder})`; match → journal release, remove.
   - `pub fn heartbeat(&mut self, slot_id: &str, owner: &LeaseOwner) ->
     Result<Option<LeaseRecord>, BusyError>` — absent → `Ok(None)`; mismatch → `Err(Busy)`;
     match → `heartbeat_at_utc = now`, journal heartbeat, `Ok(Some(record))`.
   - `pub fn sweep(&mut self, now_utc: &str) -> Vec<PeremptionEvent>` — every held record
     with `is_stale(now_utc, &hb)`: journal `reclaim` (new_owner = None), remove, collect
     event; return the Vec (NOT enqueued pending — ANSWER 7).
   - `pub fn events(&mut self) -> Vec<PeremptionEvent>` — `std::mem::take(&mut self.pending)`.
   - `pub fn held(&self, slot_id: &str) -> Option<&LeaseRecord>` — read-only observer
     (tests + daemon introspection).
7. **`src/lane.rs`** (private `mod lane`) — `pub fn lane_allowed(tier: &str) ->
   Result<(), String>`: normalize `trim().to_ascii_lowercase()`; `local|free|mid|premium` →
   `Ok(())`; `"droid"` → `Err("lane tier 'droid' is forbidden (O2: no droid lanes)".into())`;
   anything else → `Err(format!("unknown lane tier '{tier}' (expected local|free|mid|premium)"))`.
   Doc: replicated from `caddis-router/src/lane.rs LaneTier::parse` by card order — do NOT
   depend on the router crate; if the vocabulary ever moves, the fix lands in BOTH copies
   (copies law, like TLS in deliberate).
8. **`src/lib.rs`** — crate docs citing CARD-BITYNAS-1; `pub mod lease; pub mod store;`
   `mod journal; mod lane; mod rfc3339;` (private); root re-exports
   `pub use lease::{BusyError, LeaseOwner, LeaseRecord, PeremptionEvent, DEFAULT_TTL_S};`
   `pub use lane::lane_allowed; pub use store::LeaseStore;`. Include the RUNNABLE doctest
   (acceptance): temp dir named `bitynas-doc-{pid}-{nanos}` (house pattern from
   `rotate_tests.rs:48`), open store, owner A claims `"gpu-0"` on `"premium"` → `is_ok`;
   owner B claims same slot → `unwrap_err()`, assert `err.holder.session_id == "ses-A"`.
   Best-effort cleanup of the temp file.
9. **`tests/lease_tests.rs`** — 7 integration tests, RED-first (each must fail on a
   plausible bug: lost holder identity, silent preemption, index loss, owner confusion).
   Unique temp dir per test (`bitynas-t-{name}-{pid}-{nanos}`) so parallel runs don't
   collide. Helper `mk_owner(ses: &str) -> LeaseOwner`; helper `seed_stale_row(path,
   slot_id, hb_utc = "2020-01-01T00:00:00Z", ses = "ses-old")` writing one hand-made claim
   row via `fs::write`+append — this doubles as an external contract test of the row format.
   1. `double_claim_same_slot_second_gets_busy_with_holder_identity` — A claims `gpu-0`;
      B claims `gpu-0` → `Err`, `holder.session_id/host/pid` == A's triple; same-owner
      second claim also `Err` (refresh is heartbeat's job).
   2. `ttl_expiry_sweep_frees_slot_and_emits_peremption` — A claims; parse
      `rec.heartbeat_at_utc`, `sweep(hb + ttl + 1s as RFC3339)` → exactly 1 event
      (`cause == "ttl_expired"`, `previous.session_id == A`, `new_owner.is_none()`), slot
      free (`held()` → None), B can now claim.
   3. `stale_slot_is_reclaimed_by_claim_and_event_is_pending` — `seed_stale_row`, `open`,
      `events()` empty, B claims that slot → `Ok`; `events()` now yields 1 event with
      `new_owner == Some(B)`, `previous.session_id == "ses-old"`; A's `release` → `Err`
      (B holds it now).
   4. `wrong_owner_release_is_rejected_right_owner_succeeds` — A claims; B releases →
      `Err` (holder == A); A releases → `Ok`; `held()` → None; B claims → `Ok`.
   5. `heartbeat_refresh_keeps_lease_alive_across_sweep` — seed stale row, open, A
      heartbeats → `Ok(Some(rec))` with fresh `heartbeat_at_utc` (parse it, must be ≥ 2026);
      `sweep("2021-01-01T00:00:00Z")` → 0 events (old deadline no longer applies);
      `sweep(fresh_hb + ttl + 1s)` → 1 event. Both directions deterministic, no sleeps.
   6. `journal_reopen_rebuilds_index_and_sweeps_only_stale` — via store: A claims `gpu-1`,
      B claims `gpu-2` (two live records appended); append one seeded stale row; drop
      store; `open` same path → `held("gpu-1"/"gpu-2")` intact with correct identities,
      `unreadable() == 0`; `sweep(now)` frees ONLY the stale one (1 event); the live two
      still held; releasing `gpu-2` with A's owner → `Err` naming B (identity survived
      the crash).
   7. `lane_allowed_o2_droid_refused_closed_vocabulary` — `Err` for `"droid"`, `"DROID"`,
      `" droid "` (message mentions droid + O2); `Ok` for `local/free/Mid/" premium "`;
      `Err` for `"banana"` and `""` (router parity: vocabulary is closed).
10. **Verify + commit** — run VERIFY below until green (fix code, never tests, on red);
    `wc -l` gate ≤ 280 on every touched `.rs`; then stage NAMED paths only —
    `git add Cargo.toml crates/caddis-bitynas` (never `git add -A`; the tree has other
    writers) — and commit on `bitynas/h-1-lease`, message house style (see `git log`):
    `bitynas: lease-core H1 (CARD-BITYNAS-1) — LeaseStore over append-only pool/leases.jsonl …`
    with a body naming the never-silent preemption law, the O2 guard replication, and the
    std-only-beyond-serde law. NO push, NO MR.

## DONE-WHEN

- `cargo test -p caddis-bitynas` exits 0 and reports ≥ 6 tests (7 integration + rfc3339/
  journal unit tests + the lib.rs doctest — well past the bar).
- The doctest in `lib.rs` demonstrates double-claim refusal and passes.
- Root `Cargo.toml` members contain `crates/caddis-bitynas` (append-only edit) and the
  workspace manifest still parses for this crate's scope.
- All five contract items (`LeaseRecord`, `LeaseStore`, `BusyError`, `PeremptionEvent`,
  `lane_allowed`) are public per the signatures above; `pub mod lease; pub mod store;` exist.
- Every touched file ≤ 280 lines.
- One local commit on branch `bitynas/h-1-lease` containing exactly `Cargo.toml` +
  `crates/caddis-bitynas/**` (PLAN.md included); no push, no MR.

## VERIFY

From `E:\ClaudeToolbox\caddis` (cargo directly, NEVER npx; scoped to this crate —
workspace-wide validation is the main agent's job, siblings are mid-flight):

```
cargo test -p caddis-bitynas            # exit 0; count unit+integration+doctest ≥ 6
cargo test -p caddis-bitynas -- --nocapture 2>&1 | findstr /C:"test result"   # test lines
bash -lc 'for f in crates/caddis-bitynas/src/*.rs crates/caddis-bitynas/tests/*.rs; do printf "%s %s\n" "$(wc -l < "$f")" "$f"; done'   # every count ≤ 280
git status --short                      # only Cargo.toml + crates/caddis-bitynas staged/committed
git log --oneline -1                    # the bitynas H1 commit on bitynas/h-1-lease
```

RED-first spot-check before trusting green (run once, mentally or for real): flip
`is_stale` to `>=`, drop the `pending.push`, or skip journaling `heartbeat` — tests 2, 3/7,
and 5 must go red respectively.

## RISKS

1. **Two processes open the same journal concurrently** (single-writer contract violated) →
   lost claims. Mitigation: contract documented ON `LeaseStore`; the daemon organ is the
   single writer; a later card can promote caddis-core's lock law if multi-writer ever
   becomes real. Do not silently "fix" this here.
2. **Torn tail row** (crash mid-append) → on reopen it is counted in `unreadable()` and
   skipped, never parsed, never a panic (ledger CARD-0108 law: max/last-line traps avoided
   — there is no counter here to recover, only a fold).
3. **Wall-clock skew** makes a live lease look stale → wrongful preemption. Mitigation:
   `is_stale` fail-closed on parse errors; strict `>` boundary; tests never sit near
   boundaries (offsets ≥ 1 s from ttl).
4. **Root Cargo.toml edit collides with the H2 sibling** → both append members. Mitigation:
   append-only single entry, re-read immediately before editing, stage named paths only.
5. **Workspace-red confusion**: `cargo test --workspace` may fail on the sibling's
   half-landed crate. Mitigation: VERIFY is scoped to `-p caddis-bitynas`; project-wide
   validation is the main agent's single post-landing pass.
6. **Hand-rolled JSON drift**: forgetting the `esc()` law lets a newline end a record.
   Mitigation: `esc` replicated verbatim from `ledger_row.rs`; journal unit test feeds
   control chars through encode→parse.
7. **Double event delivery** (sweep return AND `events()`) would double-report preemptions
   to the daemon. Mitigation: single delivery path per operation, stated in ANSWER 7 and in
   the doc comments of both methods.
8. **`unix_from` ignores offset suffixes** (`+02:00` treated as Z). All stamps this crate
   writes are `Z`; parser accepts suffixed input lossily — documented on the fn; wrong
   offsets from foreign writers are out of scope for H1.

## STEERING AMENDMENT (executed 2026-08-30, orchestrator IRC mid-card)

The "std-only beyond serde" constraint was narrowed mid-execution: workspace
deps ARE allowed (no NEW external crates). Executed deltas:

1. `src/rfc3339.rs` (planned step 3) — DELETED before commit; time math is
   `caddis_organs::util_time::{iso8601_now, unix_from_iso8601}` via a path
   dep. The Hinnant replica never shipped.
2. `src/json.rs` hand-rolled scanner + the `esc()` replica (planned step 5)
   — never shipped; journal rows are serde_json `to_string`/`from_str`
   (`#[serde(tag = "op", rename_all = "snake_case")]`, record flattened into
   the claim row). Row strictness is serde's (torn/unknown-op rows fail
   parse and are counted unreadable; unknown extra fields are tolerated)
   instead of the planned exact-key-set law.
3. `caddis-core` untouched: `esc()` stays `pub(crate)`, no shared-file edit
   was needed once serde_json took both directions.
4. Test 2's sweep time uses fixed RFC3339 constants (2020-01-01T00:15:00Z /
   00:15:01Z) around the SEEDED row instead of parsing a live claim's hb —
   deterministic, and it pins the strict-`>` boundary exactly at ttl.

DONE-WHEN unchanged and met: `cargo test -p caddis-bitynas` exit 0, 14
tests (6 unit + 7 integration + 1 doctest), all files <= 280 lines, single
local commit on `bitynas/h-1-lease`, no push, no MR. RED spot-checks ran
FOR REAL: `is_stale` flipped to `>=` reddens test 2; `pending.push` dropped
reddens test 3; heartbeat row unjournaled reddens test 5 (reopen assert).
