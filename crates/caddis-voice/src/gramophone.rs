//! gramophone.rs — the P3 core: say-queue, WAV cache, drop-ledger, idle-clock.
//!
//! Ports four PROVEN daemon organs (peluda_voice — the invariant source; the
//! docstrings below name the exact module each behavior came from):
//!
//! - [`SayQueue`] — `scheduler.py`'s queue policy, organ-shaped: the 2 s
//!   same-key coalesce window (critical items exempt), the hard cap of 24
//!   with OLDEST-NON-CRITICAL eviction (critical items are never the loss
//!   valve and never refused), per-class due delays (0 / 0.1 / 0.5 s), and
//!   CUE-ONLY staleness measured on the idle clock — narration is never
//!   aged out. The daemon's `_COALESCE_EVENTS` burst class has no organ
//!   equivalent yet (the event lane is P4+); say lines coalesce on
//!   `(label, text)`.
//! - [`IdleClock`] — `idle_clock.py` verbatim: waiting behind live speech
//!   is the queue doing its job, never staleness; only SILENT time counts
//!   against a message, and a single utterance past the 180 s backstop is
//!   reported as a wedge so busy-credit can never hide a hang.
//! - [`DropLedger`] — `drop_ledger.py`: a dropped message must be LOUD.
//!   Per-reason counts, LOSSY reasons (operator genuinely lost
//!   information) distinguished from by-design drops (coalesced: an
//!   identical line WAS spoken), and every undelivered narration's text
//!   appended to a JSONL file so an unspoken message still leaves
//!   something readable behind.
//! - [`WavCache`] — `tts.py`'s proven cache key (sha256 over
//!   `text:voice:engine:rate:pitch:length_scale:phrase_pack_version`,
//!   first 24 hex chars) over a byte-budget LRU with honest counters.
//!
//! The dispatch loop (adapter render through the GA3 breaker, language
//! routing) and the killable play child are the NEXT slices; this module
//! owns the policy that decides what gets spoken, what gets dropped, and
//! what the operator can see about it. Pure arithmetic on a caller-supplied
//! f64 clock (seconds) — fully deterministic under test; the caller (the
//! httpd worker) holds the lock, exactly like the daemon's worker thread.

use crate::adapter::AudioFormat;
use crate::sha256::sha256_hex;
use std::collections::{BTreeMap, HashMap};
use std::path::PathBuf;

/// Same-key burst window (scheduler.py: synchronous debounce; a single
/// event after a quiet period always passes).
pub const COALESCE_WINDOW_S: f64 = 2.0;

/// The ONLY loss valve for narration (scheduler.py `queue_hard_max`): must
/// hold a whole multi-part report, because speech is never aged out.
pub const QUEUE_HARD_MAX: usize = 24;

/// Staleness budget for transient CUES, in seconds the system spent SILENT
/// (scheduler.py `queue_max_age_s`).
pub const STALE_AGE_S: f64 = 8.0;

/// A single utterance longer than this is a hang, not speech
/// (idle_clock.py `DEFAULT_MAX_UTTERANCE_S`).
pub const MAX_UTTERANCE_S: f64 = 180.0;

/// Window in which an undelivered narration counts as "recent" for health
/// (drop_ledger.py `recently_undelivered`).
pub const UNDELIVERED_WINDOW_S: f64 = 300.0;

/// Per-class due delay (scheduler.py `{0: 0.0, 1: 0.1, 2: 0.5}`).
pub fn due_delay(priority: u8) -> f64 {
    match priority {
        0 => 0.0,
        1 => 0.1,
        _ => 0.5,
    }
}

// ---------------------------------------------------------------------------
// IdleClock — idle_clock.py, verbatim port
// ---------------------------------------------------------------------------

/// Tracks cumulative speaking time so queue age can exclude it.
///
/// The invariant (the daemon bug this class closed): a message queued
/// behind a message being spoken "aged" in wall-clock while the system did
/// exactly what it should — with an 8 s staleness budget against ~15 s
/// service time, loss was STRUCTURAL; every multi-line report lost every
/// line after the first. Only time the system spent SILENT counts against
/// a message; time inside playback is credited back.
///
/// Busy-credit must never become somewhere a hang can hide: a single
/// utterance that outruns `max_utterance_s` stops counting as speech and
/// is reported as a wedge ([`IdleClock::wedged`]).
#[derive(Debug, Clone)]
pub struct IdleClock {
    busy_total: f64,
    busy_since: Option<f64>,
    /// The 180 s backstop; a test may shrink it, production never does.
    max_utterance_s: f64,
}

impl IdleClock {
    pub fn new() -> Self {
        IdleClock {
            busy_total: 0.0,
            busy_since: None,
            max_utterance_s: MAX_UTTERANCE_S,
        }
    }

    /// TEST LANE ONLY: shrink the utterance backstop.
    pub fn with_max_utterance(mut self, max_s: f64) -> Self {
        self.max_utterance_s = max_s;
        self
    }

    /// Playback is starting. Re-entry keeps the EARLIEST start so a
    /// nested/duplicated call cannot shorten a wedge.
    pub fn start_speaking(&mut self, now: f64) {
        if self.busy_since.is_none() {
            self.busy_since = Some(now);
        }
    }

    /// Playback finished; fold the utterance into the running total.
    pub fn stop_speaking(&mut self, now: f64) {
        if let Some(since) = self.busy_since.take() {
            self.busy_total += (now - since).max(0.0);
        }
    }

    pub fn speaking(&self) -> bool {
        self.busy_since.is_some()
    }

    /// Busy seconds INCLUDING the utterance in progress. Counting the
    /// in-flight utterance is what makes a mid-speech check correct: a
    /// check that only saw completed utterances would treat the 15 s it is
    /// currently speaking as idle time.
    pub fn busy_now(&self, now: f64) -> f64 {
        match self.busy_since {
            None => self.busy_total,
            Some(since) => self.busy_total + (now - since).max(0.0),
        }
    }

    /// Seconds the system was SILENT since `since`, given the `busy_at`
    /// snapshot taken at that same moment ([`SayItem::busy_at_enqueue`]).
    pub fn idle_wait(&self, since: f64, busy_at: f64, now: f64) -> f64 {
        ((now - since) - (self.busy_now(now) - busy_at)).max(0.0)
    }

    /// True when the current utterance has outrun the cap — the worker is
    /// not speaking, it is stuck, and must be recovered.
    pub fn wedged(&self, now: f64) -> bool {
        match self.busy_since {
            None => false,
            Some(since) => (now - since) > self.max_utterance_s,
        }
    }
}

impl Default for IdleClock {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// DropLedger — drop_ledger.py port
// ---------------------------------------------------------------------------

/// Drop reasons where the operator genuinely LOST information
/// (drop_ledger.py `LOSSY_REASONS`). The rest are drops by design and the
/// content still reached him: `coalesced` — an identical line WAS spoken.
/// Counting those as undelivered would raise a false alarm on every
/// correct de-duplication, and an alarm that cries wolf is one the
/// operator learns to ignore.
pub const LOSSY_REASONS: [&str; 5] = [
    "stale_age",
    "cap_overflow",
    "stall",
    "render_error",
    "process_error",
];

/// The fields /health exposes so a drop is visible from outside.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct DropHealth {
    pub total: u64,
    pub undelivered: u64,
    pub last_undelivered_ts: Option<f64>,
    /// Per-reason lifetime counts, deterministic order.
    pub by_reason: BTreeMap<String, u64>,
    /// Count of failed ledger-file writes (the ledger itself never fails
    /// the caller; this is how a broken path stays visible).
    pub persist_errors: u64,
}

/// Counts, describes and persists every dropped event. Before this organ
/// existed in the daemon, a drop bumped a bare `dropped_total` and nothing
/// else — 43 discarded status reports with every human-readable field
/// saying "fine". For an operator who LISTENS, a dropped message is
/// indistinguishable from "nothing happened": the worst failure shape this
/// system has. The ledger makes it loud.
#[derive(Debug, Clone)]
pub struct DropLedger {
    total: u64,
    undelivered: u64,
    last_undelivered_ts: Option<f64>,
    by_reason: BTreeMap<String, u64>,
    persist_errors: u64,
    undelivered_path: Option<PathBuf>,
}

impl DropLedger {
    /// `undelivered_path`: JSONL file beside the event log carrying the
    /// same content class (spoken text) — an unreadable-behind message
    /// still leaves something readable. `None` = count only (tests).
    pub fn new(undelivered_path: Option<PathBuf>) -> Self {
        DropLedger {
            total: 0,
            undelivered: 0,
            last_undelivered_ts: None,
            by_reason: BTreeMap::new(),
            persist_errors: 0,
            undelivered_path,
        }
    }

    /// True when a NARRATION went unheard inside the window — the
    /// operator-facing alarm signal.
    pub fn recently_undelivered(&self, now: f64) -> bool {
        match self.last_undelivered_ts {
            Some(ts) => (now - ts) < UNDELIVERED_WINDOW_S,
            None => false,
        }
    }

    /// Register one drop. NEVER fails the caller: a failure to write the
    /// ledger is counted ([`DropHealth::persist_errors`]), never raised —
    /// the drop path must not grow new failure modes.
    pub fn record(&mut self, item: &SayItem, reason: &str, now: f64) {
        self.total += 1;
        *self.by_reason.entry(reason.to_string()).or_insert(0) += 1;
        if LOSSY_REASONS.contains(&reason) {
            self.undelivered += 1;
            self.last_undelivered_ts = Some(now);
            self.persist_undelivered(item, reason, now);
        }
    }

    fn persist_undelivered(&mut self, item: &SayItem, reason: &str, now: f64) {
        let Some(path) = &self.undelivered_path else {
            return;
        };
        let line = format!(
            "{{\"ts\":{:.3},\"reason\":\"{}\",\"label\":\"{}\",\"text\":\"{}\"}}\n",
            now,
            json_escape(reason),
            json_escape(&item.label),
            json_escape(&item.text),
        );
        // Append-or-create; a broken path bumps persist_errors and the
        // counts above still stand (in-memory truth first).
        let wrote = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
            .and_then(|mut f| std::io::Write::write_all(&mut f, line.as_bytes()));
        if wrote.is_err() {
            self.persist_errors += 1;
        }
    }

    pub fn health(&self) -> DropHealth {
        DropHealth {
            total: self.total,
            undelivered: self.undelivered,
            last_undelivered_ts: self.last_undelivered_ts,
            by_reason: self.by_reason.clone(),
            persist_errors: self.persist_errors,
        }
    }
}

/// Escape one JSON string body (drop_ledger.py writes json.dumps; the
/// organ hand-rolls the same contract — quotes, backslash, and C0 controls
/// as \u00XX).
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// SayQueue — scheduler.py's queue policy, organ-shaped
// ---------------------------------------------------------------------------

/// One admitted say line. `narration` is the class distinction the
/// staleness policy keys on: narration is NEVER aged out, cues carry the
/// 8 s silent-seconds budget.
#[derive(Debug, Clone, PartialEq)]
pub struct SayItem {
    /// Who/what this line is for (drop-ledger naming).
    pub label: String,
    pub text: String,
    /// true = narration (never aged); false = transient cue (stale-able).
    pub narration: bool,
    /// 0 = critical: never coalesced, never evicted, due immediately.
    pub priority: u8,
    /// Admission sequence — FIFO order within equal priority.
    pub seq: u64,
    /// Caller-clock seconds when admitted.
    pub enqueued_at: f64,
    /// [`IdleClock::busy_now`] snapshot at admission — the reference the
    /// staleness arithmetic subtracts.
    pub busy_at_enqueue: f64,
    /// Earliest pop time: `enqueued_at + due_delay(priority)`.
    pub due_at: f64,
}

/// The synchronous admission verdict.
#[derive(Debug, Clone, PartialEq)]
pub enum Admission {
    /// Enqueued.
    Queued,
    /// Same `(label, text)` burst inside the window (non-critical): an
    /// identical line WILL be spoken — not operator loss.
    Coalesced,
    /// Queue was at the hard cap: the OLDEST non-critical item was evicted
    /// so this one fits. The caller records the evicted item in the
    /// [`DropLedger`] (`cap_overflow`) — a critical-only queue is never
    /// evicted from and never refused.
    Evicted(Box<SayItem>),
}

/// The bounded say-queue with the daemon's proven admission and ordering
/// policy. Not thread-safe by itself — the dispatch slice's worker owns
/// the lock, exactly like the daemon's worker thread owned the scheduler
/// lock.
#[derive(Debug, Clone)]
pub struct SayQueue {
    /// Ready-ordered on pop by `(priority, seq)`; small by construction
    /// (hard cap 24, plus any critical overflow).
    items: Vec<SayItem>,
    /// `(label, text)` -> last ACCEPT ts (scheduler.py `last_seen`).
    /// Entries older than the window are dead weight; the daemon never
    /// pruned (keys bounded by distinct say lines in practice), but an
    /// organ that lives for months takes the cheap guard.
    last_seen: HashMap<(String, String), f64>,
    seq: u64,
    hard_max: usize,
    coalesce_window_s: f64,
    stale_age_s: f64,
}

impl SayQueue {
    pub fn new() -> Self {
        SayQueue {
            items: Vec::new(),
            last_seen: HashMap::new(),
            seq: 0,
            hard_max: QUEUE_HARD_MAX,
            coalesce_window_s: COALESCE_WINDOW_S,
            stale_age_s: STALE_AGE_S,
        }
    }

    /// TEST LANE ONLY: shrink the policy bounds.
    pub fn with_bounds(hard_max: usize, coalesce_window_s: f64, stale_age_s: f64) -> Self {
        SayQueue {
            hard_max,
            coalesce_window_s,
            stale_age_s,
            ..SayQueue::new()
        }
    }

    /// Admit one line. `busy_now` is [`IdleClock::busy_now`] at `now` —
    /// snapshotted onto the item so staleness later subtracts exactly the
    /// speech that happened AFTER admission.
    pub fn submit(
        &mut self,
        label: &str,
        text: &str,
        narration: bool,
        priority: u8,
        now: f64,
        busy_now: f64,
    ) -> Admission {
        // Same-key burst inside the window: coalesce NON-critical items
        // only; a critical line is never silently swallowed by a burst
        // (scheduler.py: `prio > 0` guard). A reject does NOT refresh
        // last_seen — the daemon updated it only on accept, so a burst of
        // N duplicates inside the window costs one spoken line, not one
        // per two submissions.
        let key = (label.to_string(), text.to_string());
        if priority > 0 {
            if let Some(&seen) = self.last_seen.get(&key) {
                if now - seen < self.coalesce_window_s {
                    return Admission::Coalesced;
                }
            }
        }
        self.last_seen.insert(key, now);
        self.prune_last_seen(now);

        // Hard cap: evict the OLDEST non-critical item (earliest due, then
        // priority, then seq — the daemon's `noncrit[0]` in heap order).
        // Critical items are never evicted and never refused: an
        // all-critical queue grows past the cap rather than losing one.
        let evicted = if self.items.len() >= self.hard_max {
            self.evict_oldest_noncritical().map(Box::new)
        } else {
            None
        };

        let seq = self.seq;
        self.seq += 1;
        self.items.push(SayItem {
            label: label.to_string(),
            text: text.to_string(),
            narration,
            priority,
            seq,
            enqueued_at: now,
            busy_at_enqueue: busy_now,
            due_at: now + due_delay(priority),
        });

        match evicted {
            Some(item) => Admission::Evicted(item),
            None => Admission::Queued,
        }
    }

    /// The seq of the most recently PUSHED item (undefined before the
    /// first push). The service layer keys per-item routing side-data
    /// (R-B speech path) by this — `submit` owns the counter, so the
    /// caller cannot race it.
    pub fn last_seq(&self) -> u64 {
        self.seq.saturating_sub(1)
    }

    fn evict_oldest_noncritical(&mut self) -> Option<SayItem> {
        let idx = self
            .items
            .iter()
            .enumerate()
            .filter(|(_, i)| i.priority > 0)
            .min_by(|a, b| {
                a.1.due_at
                    .partial_cmp(&b.1.due_at)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then(a.1.priority.cmp(&b.1.priority))
                    .then(a.1.seq.cmp(&b.1.seq))
            })
            .map(|(idx, _)| idx)?;
        Some(self.items.remove(idx))
    }

    /// The daemon's `last_seen` grew unbounded; the organ prunes entries
    /// outside the coalesce window once the map passes 4096 rows — the
    /// only divergence, and it cannot change behavior: a pruned entry is
    /// by definition older than the window and could never coalesce.
    fn prune_last_seen(&mut self, now: f64) {
        if self.last_seen.len() <= 4096 {
            return;
        }
        self.last_seen
            .retain(|_, ts| now - *ts < self.coalesce_window_s);
    }

    /// Pop the next line due for speech. Stale CUES are dropped through
    /// the ledger and the scan continues — one stale cue never hides the
    /// line behind it. NARRATION is never aged out: the heap cap is its
    /// only loss valve. `None` = nothing due (an empty queue or a head
    /// still inside its due delay is the queue correctly waiting, not a
    /// stall).
    pub fn pop(&mut self, now: f64, clock: &IdleClock, ledger: &mut DropLedger) -> Option<SayItem> {
        loop {
            // Best READY item: priority first, FIFO within a class.
            let best = self
                .items
                .iter()
                .enumerate()
                .filter(|(_, i)| now >= i.due_at)
                .min_by(|a, b| a.1.priority.cmp(&b.1.priority).then(a.1.seq.cmp(&b.1.seq)))
                .map(|(idx, _)| idx)?;
            let item = self.items.remove(best);
            // Staleness budget: CUES only, in SILENT seconds. Waiting
            // behind live speech is the queue doing its job.
            if !item.narration {
                let waited = clock.idle_wait(item.enqueued_at, item.busy_at_enqueue, now);
                if waited > self.stale_age_s {
                    ledger.record(&item, "stale_age", now);
                    continue;
                }
            }
            return Some(item);
        }
    }

    /// Drop every queued-but-unspoken item through the normal drop path
    /// (stand-down/shutdown). Returns the count. Emptying the queue any
    /// other way orphans accounting — every item passes the ledger.
    pub fn drop_pending(&mut self, reason: &str, now: f64, ledger: &mut DropLedger) -> usize {
        let pending: Vec<SayItem> = std::mem::take(&mut self.items);
        let n = pending.len();
        for item in &pending {
            ledger.record(item, reason, now);
        }
        n
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

impl Default for SayQueue {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// WavCache — tts.py's proven key over a byte-budget LRU
// ---------------------------------------------------------------------------

/// A cache hit. Cloned out (playback needs ownership anyway).
#[derive(Debug, Clone, PartialEq)]
pub struct CachedAudio {
    pub bytes: Vec<u8>,
    pub format: AudioFormat,
    /// Render cost of the ORIGINAL render (telemetry: a cache hit's "cost
    /// avoided" is the soak story).
    pub rendered_ms: u128,
}

#[derive(Debug, Clone)]
struct CacheEntry {
    audio: CachedAudio,
    last_used: u64,
}

/// Lifetime counters — the honest shape (a cache that cannot say what it
/// did is a cache nobody trusts).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
    pub stores: u64,
    pub evictions: u64,
    /// Oversized payloads refused at the door (bytes > the whole budget).
    pub rejects: u64,
    pub entries: usize,
    pub bytes: usize,
    pub max_bytes: usize,
}

/// The pre-render WAV cache. Key = the daemon's PROVEN composite
/// ([`wav_cache_key`]): any change to text, voice, engine, prosody or the
/// phrase pack is a different utterance. Byte-budgeted LRU: the entry
/// untouched longest dies first, an entry bigger than the whole budget is
/// refused (never thrash the cache for one giant render).
#[derive(Debug, Clone)]
pub struct WavCache {
    max_bytes: usize,
    bytes: usize,
    entries: HashMap<String, CacheEntry>,
    tick: u64,
    hits: u64,
    misses: u64,
    stores: u64,
    evictions: u64,
    rejects: u64,
}

/// The daemon's cache key, verbatim shape:
/// `sha256("{text}:{voice}:{engine}:{rate}:{pitch}:{length_scale}:{phrase_pack_version}")`
/// truncated to 24 hex chars (tts.py `_wav_path`). Floats format via Rust
/// `{}` — the organ's cache is a fresh namespace; only STABILITY within it
/// matters.
pub fn wav_cache_key(
    text: &str,
    voice: &str,
    engine: &str,
    rate: &str,
    pitch: &str,
    length_scale: f64,
    phrase_pack_version: &str,
) -> String {
    let composite =
        format!("{text}:{voice}:{engine}:{rate}:{pitch}:{length_scale}:{phrase_pack_version}");
    sha256_hex(composite.as_bytes())[..24].to_string()
}

impl WavCache {
    /// `max_bytes` = the whole-cache byte budget (an in-memory organ does
    /// not pay the daemon's disk dance; the budget law stays).
    pub fn new(max_bytes: usize) -> Self {
        WavCache {
            max_bytes,
            bytes: 0,
            entries: HashMap::new(),
            tick: 0,
            hits: 0,
            misses: 0,
            stores: 0,
            evictions: 0,
            rejects: 0,
        }
    }

    pub fn get(&mut self, key: &str) -> Option<CachedAudio> {
        self.tick += 1;
        let tick = self.tick;
        match self.entries.get_mut(key) {
            Some(e) => {
                e.last_used = tick;
                self.hits += 1;
                Some(e.audio.clone())
            }
            None => {
                self.misses += 1;
                None
            }
        }
    }

    /// Store one render. Returns false (and counts a reject) when the
    /// payload alone exceeds the whole budget — evicting everything for
    /// one entry is a cache that forgot its job.
    pub fn put(&mut self, key: &str, audio: CachedAudio) -> bool {
        if audio.bytes.len() > self.max_bytes {
            self.rejects += 1;
            return false;
        }
        // Replace-in-place: refund the old bytes first.
        if let Some(old) = self.entries.remove(key) {
            self.bytes -= old.audio.bytes.len();
        }
        while self.bytes + audio.bytes.len() > self.max_bytes {
            match self
                .entries
                .iter()
                .min_by_key(|(_, e)| e.last_used)
                .map(|(k, _)| k.clone())
            {
                Some(lru) => {
                    if let Some(old) = self.entries.remove(&lru) {
                        self.bytes -= old.audio.bytes.len();
                        self.evictions += 1;
                    }
                }
                None => break,
            }
        }
        self.bytes += audio.bytes.len();
        self.tick += 1;
        let last_used = self.tick;
        self.entries
            .insert(key.to_string(), CacheEntry { audio, last_used });
        self.stores += 1;
        true
    }

    pub fn stats(&self) -> CacheStats {
        CacheStats {
            hits: self.hits,
            misses: self.misses,
            stores: self.stores,
            evictions: self.evictions,
            rejects: self.rejects,
            entries: self.entries.len(),
            bytes: self.bytes,
            max_bytes: self.max_bytes,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(label: &str, text: &str, priority: u8, narration: bool) -> SayItem {
        SayItem {
            label: label.into(),
            text: text.into(),
            narration,
            priority,
            seq: 0,
            enqueued_at: 0.0,
            busy_at_enqueue: 0.0,
            due_at: 0.0,
        }
    }

    // -----------------------------------------------------------------
    // IdleClock — the arithmetic IS the invariant
    // -----------------------------------------------------------------

    #[test]
    fn idle_wait_excludes_speech_time() {
        let mut c = IdleClock::new();
        // Message enqueued at t=0 (busy 0). Speech 0..15 (a whole
        // narration blocks the worker), then silence; check at t=16.
        c.start_speaking(0.0);
        c.stop_speaking(15.0);
        assert_eq!(c.busy_total, 15.0);
        assert!(!c.speaking());
        let waited = c.idle_wait(0.0, 0.0, 16.0);
        assert!(
            (waited - 1.0).abs() < 1e-9,
            "16s wall - 15s speech = 1s silent, got {waited}"
        );
    }

    #[test]
    fn idle_wait_counts_silence_fully() {
        let mut c = IdleClock::new();
        c.start_speaking(0.0);
        c.stop_speaking(2.0);
        // Silent 2..10 after speaking 0..2: a t=0 enqueue with busy_at=0
        // has waited 10 - 2 = 8 silent seconds.
        assert!((c.idle_wait(0.0, 0.0, 10.0) - 8.0).abs() < 1e-9);
    }

    #[test]
    fn reentry_keeps_earliest_start() {
        let mut c = IdleClock::new();
        c.start_speaking(5.0);
        c.start_speaking(9.0); // nested/duplicate call must not shorten
        c.stop_speaking(7.0);
        assert!((c.busy_total - 2.0).abs() < 1e-9);
    }

    #[test]
    fn stop_clamps_negative_and_busy_now_includes_inflight() {
        let mut c = IdleClock::new();
        c.stop_speaking(10.0); // stop without start: no-op, no panic
        assert_eq!(c.busy_total, 0.0);
        c.start_speaking(10.0);
        assert!((c.busy_now(13.0) - 3.0).abs() < 1e-9);
        assert!(c.speaking());
    }

    #[test]
    fn wedged_when_utterance_outruns_backstop() {
        let mut c = IdleClock::new().with_max_utterance(30.0);
        c.start_speaking(0.0);
        assert!(!c.wedged(29.9));
        assert!(c.wedged(30.1));
        c.stop_speaking(31.0);
        assert!(!c.wedged(100.0)); // nothing in flight
    }

    // -----------------------------------------------------------------
    // DropLedger — loud drops, quiet by-design drops
    // -----------------------------------------------------------------

    #[test]
    fn ledger_counts_lossy_and_design_separately() {
        let mut l = DropLedger::new(None);
        l.record(&item("bee", "job done", 1, false), "coalesced", 10.0);
        l.record(&item("bee", "job failed", 1, true), "stale_age", 11.0);
        l.record(
            &item("bee", "render blew up", 2, true),
            "render_error",
            12.0,
        );
        let h = l.health();
        assert_eq!(h.total, 3);
        assert_eq!(h.undelivered, 2); // coalesced is NOT loss
        assert_eq!(h.by_reason.get("coalesced"), Some(&1));
        assert_eq!(h.by_reason.get("stale_age"), Some(&1));
        assert_eq!(h.last_undelivered_ts, Some(12.0));
        assert!(l.recently_undelivered(12.0 + 299.0));
        assert!(!l.recently_undelivered(12.0 + 301.0));
    }

    #[test]
    fn ledger_persists_undelivered_text() {
        let dir = std::env::temp_dir();
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let path = dir.join(format!("caddis-gram-ledger-{nanos}.jsonl"));
        let mut l = DropLedger::new(Some(path.clone()));
        l.record(
            &item("sergeant", "say \"quoted\"\nand \\slash", 1, true),
            "cap_overflow",
            100.5,
        );
        let raw = std::fs::read_to_string(&path).unwrap();
        assert!(raw.contains("\"reason\":\"cap_overflow\""));
        assert!(raw.contains("say \\\"quoted\\\"\\nand \\\\slash"));
        assert!(raw.starts_with("{\"ts\":100.500"));
        let _ = std::fs::remove_file(&path);
    }

    #[test]
    fn ledger_survives_broken_path_without_failing_caller() {
        // A DIRECTORY as the ledger file: every write fails; the caller
        // still gets its counts and the failure stays visible.
        let mut l = DropLedger::new(Some(PathBuf::from(".")));
        l.record(&item("x", "lost words", 1, true), "stall", 5.0);
        let h = l.health();
        assert_eq!(h.total, 1);
        assert_eq!(h.undelivered, 1);
        assert_eq!(h.persist_errors, 1);
    }

    // -----------------------------------------------------------------
    // WavCache — the proven key + an honest budget
    // -----------------------------------------------------------------

    #[test]
    fn cache_key_is_daemon_shaped_and_field_sensitive() {
        let base = wav_cache_key("hello", "ryan", "piper", "+0%", "low", 1.0, "1.0.0");
        assert_eq!(base.len(), 24);
        assert!(base.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(
            base,
            wav_cache_key("hello", "ryan", "piper", "+0%", "low", 1.0, "1.0.0")
        );
        assert_ne!(
            base,
            wav_cache_key("hellO", "ryan", "piper", "+0%", "low", 1.0, "1.0.0")
        );
        assert_ne!(
            base,
            wav_cache_key("hello", "leonas", "piper", "+0%", "low", 1.0, "1.0.0")
        );
        assert_ne!(
            base,
            wav_cache_key("hello", "ryan", "leonas", "+0%", "low", 1.0, "1.0.0")
        );
        assert_ne!(
            base,
            wav_cache_key("hello", "ryan", "piper", "+5%", "low", 1.0, "1.0.0")
        );
        assert_ne!(
            base,
            wav_cache_key("hello", "ryan", "piper", "+0%", "high", 1.0, "1.0.0")
        );
        assert_ne!(
            base,
            wav_cache_key("hello", "ryan", "piper", "+0%", "low", 0.9, "1.0.0")
        );
        assert_ne!(
            base,
            wav_cache_key("hello", "ryan", "piper", "+0%", "low", 1.0, "2.0.0")
        );
    }

    fn audio(n: usize) -> CachedAudio {
        CachedAudio {
            bytes: vec![0u8; n],
            format: AudioFormat::Wav,
            rendered_ms: 2000,
        }
    }

    #[test]
    fn cache_hit_miss_and_stats() {
        let mut c = WavCache::new(1000);
        assert!(c.get("k").is_none());
        assert!(c.put("k", audio(10)));
        assert!(c.get("k").is_some());
        assert!(c.get("k").is_some());
        let s = c.stats();
        assert_eq!(
            (s.hits, s.misses, s.stores, s.evictions, s.rejects),
            (2, 1, 1, 0, 0)
        );
        assert_eq!((s.entries, s.bytes), (1, 10));
    }

    #[test]
    fn cache_evicts_lru_not_mru() {
        let mut c = WavCache::new(30);
        assert!(c.put("a", audio(10)));
        assert!(c.put("b", audio(10)));
        assert!(c.get("a").is_some()); // a is now MRU
        assert!(c.put("c", audio(10)));
        assert!(c.put("d", audio(10))); // evicts b (LRU), not a
        assert!(c.get("a").is_some());
        assert!(c.get("b").is_none());
        assert_eq!(c.stats().evictions, 1);
    }

    #[test]
    fn cache_replaces_same_key_and_refuses_oversize() {
        let mut c = WavCache::new(50);
        assert!(c.put("k", audio(40)));
        assert!(c.put("k", audio(50))); // replace: refund 40, fits
        assert_eq!(c.stats().bytes, 50);
        assert!(!c.put("huge", audio(51))); // bigger than the WHOLE budget
        assert_eq!(c.stats().rejects, 1);
        assert_eq!(c.stats().entries, 1); // cache untouched by the refusal
    }

    // -----------------------------------------------------------------
    // SayQueue — admission, ordering, cue-only staleness
    // -----------------------------------------------------------------

    fn fresh() -> (SayQueue, IdleClock, DropLedger) {
        (SayQueue::new(), IdleClock::new(), DropLedger::new(None))
    }

    #[test]
    fn coalesce_window_same_key_noncritical_only() {
        let (mut q, _, _) = fresh();
        assert_eq!(
            q.submit("bee", "job done", false, 1, 100.0, 0.0),
            Admission::Queued
        );
        assert_eq!(
            q.submit("bee", "job done", false, 1, 101.0, 0.0),
            Admission::Coalesced
        );
        // Outside the window: accepted again.
        assert_eq!(
            q.submit("bee", "job done", false, 1, 103.0, 0.0),
            Admission::Queued
        );
        // Critical (priority 0) NEVER coalesced, even in a burst.
        assert_eq!(
            q.submit("alarm", "wake up", true, 0, 100.0, 0.0),
            Admission::Queued
        );
        assert_eq!(
            q.submit("alarm", "wake up", true, 0, 100.1, 0.0),
            Admission::Queued
        );
        // Different text never coalesced.
        assert_eq!(
            q.submit("bee", "job failed", false, 1, 100.2, 0.0),
            Admission::Queued
        );
    }

    #[test]
    fn hard_cap_evicts_oldest_noncritical_never_critical() {
        let (mut q, _, _) = fresh();
        for i in 0..24 {
            let adm = q.submit("src", &format!("line {i}"), true, 2, 100.0 + i as f64, 0.0);
            assert!(matches!(adm, Admission::Queued), "line {i}");
        }
        assert_eq!(q.len(), 24);
        // 25th non-critical: oldest non-critical ("line 0") evicted.
        match q.submit("src", "line 24", true, 2, 200.0, 0.0) {
            Admission::Evicted(v) => {
                assert_eq!(v.text, "line 0");
                assert!(v.priority > 0);
            }
            other => panic!("expected eviction, got {other:?}"),
        }
        // A CRITICAL submit into the full queue evicts a non-critical too.
        match q.submit("alarm", "critical now", true, 0, 201.0, 0.0) {
            Admission::Evicted(v) => assert_eq!(v.text, "line 1"),
            other => panic!("expected eviction, got {other:?}"),
        }
        // Fill with criticals only: no eviction, no refusal.
        let mut q2 = SayQueue::with_bounds(2, 2.0, 8.0);
        for i in 0..5 {
            let adm = q2.submit(
                "alarm",
                &format!("crit {i}"),
                true,
                0,
                100.0 + i as f64,
                0.0,
            );
            assert!(matches!(adm, Admission::Queued), "crit {i}");
        }
        assert_eq!(q2.len(), 5); // cap exceeded rather than losing a critical
    }

    #[test]
    fn due_delays_priority_then_fifo_ordering() {
        let (mut q, c, mut l) = fresh();
        q.submit("a", "normal first", true, 2, 10.0, 0.0);
        q.submit("b", "high second", true, 1, 10.2, 0.0);
        q.submit("c", "critical third", true, 0, 10.4, 0.0);
        // At t=10.45: high due 10.3, critical due 10.4 — both ready,
        // critical wins on priority; normal (due 10.5) still waiting.
        let got = q.pop(10.45, &c, &mut l).unwrap();
        assert_eq!(got.text, "critical third");
        let got = q.pop(10.45, &c, &mut l).unwrap();
        assert_eq!(got.text, "high second");
        assert!(q.pop(10.45, &c, &mut l).is_none()); // normal still waiting
        let got = q.pop(10.6, &c, &mut l).unwrap();
        assert_eq!(got.text, "normal first");
        assert!(q.is_empty());
    }

    #[test]
    fn fifo_within_same_priority() {
        let (mut q, c, mut l) = fresh();
        q.submit("a", "one", true, 1, 10.0, 0.0);
        q.submit("b", "two", true, 1, 10.1, 0.0);
        assert_eq!(q.pop(20.0, &c, &mut l).unwrap().text, "one");
        assert_eq!(q.pop(20.0, &c, &mut l).unwrap().text, "two");
    }

    #[test]
    fn stale_cue_dropped_via_ledger_scan_continues() {
        let (mut q, c, mut l) = fresh();
        // A cue admitted at t=0 with busy 0; system silent the whole time.
        assert_eq!(
            q.submit("ev", "old cue", false, 2, 0.0, 0.0),
            Admission::Queued
        );
        // A narration admitted at t=9 (still fresh by construction —
        // narration never ages, and its idle wait starts at admission).
        assert_eq!(
            q.submit("ev", "real report", true, 2, 9.0, 0.0),
            Admission::Queued
        );
        // Pop at t=20: the cue waited 20 SILENT seconds (> 8) -> dropped
        // loudly; the narration behind it is served in the same pop call.
        let got = q.pop(20.0, &c, &mut l).unwrap();
        assert_eq!(got.text, "real report");
        let h = l.health();
        assert_eq!(h.total, 1);
        assert_eq!(h.by_reason.get("stale_age"), Some(&1));
        assert_eq!(h.undelivered, 1);
        assert!(q.is_empty());
    }

    #[test]
    fn cue_wait_measured_on_idle_clock_not_wall_clock() {
        let (mut q, mut c, mut l) = fresh();
        // Cue enqueued at t=0, busy snapshot 0. The system then SPEAKS
        // 0..15 (a long narration in flight).
        assert_eq!(
            q.submit("ev", "behind speech", false, 2, 0.0, 0.0),
            Admission::Queued
        );
        c.start_speaking(0.0);
        // Pop attempt at t=14: only 14 wall, 14 spoken -> idle wait 0.
        let got = q.pop(14.0, &c, &mut l).unwrap();
        assert_eq!(got.text, "behind speech");
        assert_eq!(
            l.health().total,
            0,
            "waiting behind speech is not staleness"
        );
    }

    #[test]
    fn narration_never_ages_out() {
        let (mut q, c, mut l) = fresh();
        assert_eq!(
            q.submit("r", "long report", true, 2, 0.0, 0.0),
            Admission::Queued
        );
        // A hundred silent seconds later, narration is still served.
        let got = q.pop(100.0, &c, &mut l).unwrap();
        assert_eq!(got.text, "long report");
        assert_eq!(l.health().total, 0);
    }

    #[test]
    fn drop_pending_accounts_every_item() {
        let (mut q, _, mut l) = fresh();
        q.submit("a", "one", true, 1, 0.0, 0.0);
        q.submit("a", "two", true, 1, 0.1, 0.0);
        let n = q.drop_pending("stall", 50.0, &mut l);
        assert_eq!(n, 2);
        assert!(q.is_empty());
        let h = l.health();
        assert_eq!(h.total, 2);
        assert_eq!(h.undelivered, 2); // stall is lossy
    }

    #[test]
    fn submit_snapshots_busy_at_enqueue() {
        let (mut q, mut c, mut l) = fresh();
        c.start_speaking(0.0);
        c.stop_speaking(10.0); // busy_total = 10
        c.start_speaking(12.0);
        q.submit("x", "mid speech", false, 2, 15.0, c.busy_now(15.0));
        c.stop_speaking(20.0); // busy_total = 18
                               // At t=23: wall wait 8, busy added since enqueue = 3 -> idle 5 < 8.
        assert!(q.pop(23.0, &c, &mut l).is_some());
        assert_eq!(l.health().total, 0);
    }
}
