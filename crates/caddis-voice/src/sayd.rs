//! sayd.rs — the SAY SERVICE (P3 slice c): the gramophone assembled
//! end-to-end — queue → dispatch → play child — plus the EARCON wiring
//! (QQ5: attention/done/fail + the R-B `substituted`/`degrade` motifs)
//! and the two surfaces the httpd exposes (`/say`, `/earcon`).
//!
//! Thread shape (the daemon's shape, ported): HTTP threads ADMIT under a
//! brief front lock; ONE worker owns the speaking machinery (dispatcher,
//! lanes, sink, ledger, detector) so speech serializes on a single
//! throat. The worker pops under the front lock, speaks WITHOUT it (one
//! utterance can legally take 12-16 s), then folds its counts back.
//! std::sync::Mutex (the crate is zero-dep by charter); production lock
//! paths degrade gracefully, only `say()` panics on a poisoned front —
//! a worker that died mid-speech must not be papered over.
//!
//! **The busy-echo approximation** (the one deliberate divergence):
//! admission snapshots `IdleClock::busy_now` from a value the worker
//! publishes at every front passage, not the live clock (the daemon held
//! ONE scheduler lock, so its snapshot was exact; the organ splits the
//! locks so /say stays responsive mid-utterance). The echo can only
//! UNDER-count busy seconds, and under-counting busy makes cues look
//! FRESHER — staleness drops a cue LATER, never earlier. The non-lossy
//! direction, on the only policy that reads the snapshot.
//!
//! **The chime table** (one table, this layer — say.rs point 2): the
//! dispatcher drops loudly; THIS layer decides what the operator HEARS:
//!
//! | event | motif | fires when |
//! |---|---|---|
//! | `bee.fail` | `fail` | a render-lane drop (GA3 trip / render error / no lane wired) — the audio path is alive, so he still hears that something was lost |
//! | `substitute` | `substituted` | R-B general speech spoke on a SUBSTITUTE voice |
//! | `degrade` | `degrade` | R-B gated confirm honestly degraded to silence |
//! | `attention`/`done`/… | per set | caller-fired life events (opener P4 routes these) |
//!
//! A `process_error` (the play sink itself failed) fires NO chime —
//! proven-broken audio would swallow it; the ledger row is the truth.

use crate::adapter::BreakerConfig;
use crate::config::{LabelConfig, OrganConfig};
use crate::detect::{DetectOptions, Detector, Utterance};
use crate::earcons::{synth_wav, EarconSet, EARCON_SAMPLE_RATE};
use crate::gramophone::{Admission, DropLedger, DropHealth, IdleClock, SayItem, SayQueue};
use crate::lang::Lang;
use crate::say::{Dispatcher, PlaySink, RenderLane, SayOutcome};
use crate::voiceset::{self, RouteDecision, SpeechPath};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

/// Worker wake/poll cadence — a submitted line starts draining at worst
/// this late even without the wake message (the daemon's worker poll).
const WORKER_POLL: Duration = Duration::from_millis(50);

fn wall_s() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

// ---------------------------------------------------------------------------

/// Synthesizes + caches the motif WAVs, plays them through a sink inside
/// an idle-clock bracket (a chime occupies the audio channel exactly like
/// speech does — its seconds are busy seconds, measured real).
#[derive(Default)]
pub struct EarconPlayer {
    set: EarconSet,
    wavs: HashMap<String, Vec<u8>>,
}

impl EarconPlayer {
    /// True when `event` maps to a motif (the /earcon route validates
    /// against this — unknown events are caller mistakes, refused).
    pub fn knows(event: &str) -> bool {
        EarconSet::default().event_map.contains_key(event)
    }

    /// Play one event's motif; synth-once-per-motif. Best-effort by
    /// design: a chime is an ALERT, not a delivery — its failure counts
    /// (the caller tracks `earcons_played`) but never raises.
    pub fn play(
        &mut self,
        event: &str,
        clock: &mut IdleClock,
        now_s: f64,
        sink: &mut dyn PlaySink,
    ) -> bool {
        let Some(motif_id) = self.set.event_map.get(event) else {
            return false;
        };
        let Some(motif) = self.set.motifs.get(motif_id) else {
            return false;
        };
        let wav = self
            .wavs
            .entry(motif_id.clone())
            .or_insert_with(|| synth_wav(motif, EARCON_SAMPLE_RATE));
        clock.start_speaking(now_s);
        let t0 = Instant::now();
        let ok = sink.play(wav);
        clock.stop_speaking(now_s + t0.elapsed().as_secs_f64());
        ok
    }
}

// ---------------------------------------------------------------------------
// Front stage (shared, brief locks) + worker (single owner of speaking)
// ---------------------------------------------------------------------------

/// Lifetime counters the /say response and P4 /health surface.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SayCounts {
    pub submitted: u64,
    pub coalesced: u64,
    pub evicted: u64,
    pub spoken: u64,
    pub cache_hits: u64,
    pub dropped: u64,
    pub substituted: u64,
    pub degraded: u64,
    pub earcons_played: u64,
    pub queue_len: usize,
}

/// The shared half: the queue, per-item routing side-data, counters, and
/// the busy-echo the worker refreshes at every passage (see module doc).
struct FrontStage {
    queue: SayQueue,
    paths: HashMap<u64, SpeechPath>,
    counts: SayCounts,
    busy_echo: f64,
}

impl FrontStage {
    fn new() -> Self {
        FrontStage {
            queue: SayQueue::new(),
            paths: HashMap::new(),
            counts: SayCounts::default(),
            busy_echo: 0.0,
        }
    }
}

/// Admit one line onto the front stage (worker AND tests share this —
/// one admission path, one counting path).
fn submit_stage(
    front: &mut FrontStage,
    label: &str,
    text: &str,
    narration: bool,
    priority: u8,
    path: SpeechPath,
    now_s: f64,
) -> Admission {
    let adm = front
        .queue
        .submit(label, text, narration, priority, now_s, front.busy_echo);
    front.counts.submitted += 1;
    match &adm {
        Admission::Queued => {}
        Admission::Coalesced => front.counts.coalesced += 1,
        Admission::Evicted(_) => front.counts.evicted += 1,
    }
    if let Admission::Queued | Admission::Evicted(_) = adm {
        front.paths.insert(front.queue.last_seq(), path);
    }
    front.counts.queue_len = front.queue.len();
    adm
}

/// Count deltas one spoken/dropped item produces (applied to the front
/// under its lock AFTER the audio work — the lock never spans speech).
#[derive(Default)]
struct Delta {
    spoken: u32,
    cache_hits: u32,
    dropped: u32,
    substituted: u32,
    degraded: u32,
    earcons: u32,
}

impl Delta {
    fn apply(self, front: &mut FrontStage) {
        let c = &mut front.counts;
        c.spoken += u64::from(self.spoken);
        c.cache_hits += u64::from(self.cache_hits);
        c.dropped += u64::from(self.dropped);
        c.substituted += u64::from(self.substituted);
        c.degraded += u64::from(self.degraded);
        c.earcons_played += u64::from(self.earcons);
        c.queue_len = front.queue.len();
    }
}

/// The single owner of the speaking machinery (sayd's deterministic core
/// — tests drive this directly with fake clocks; the service thread is a
/// thin shell around it).
pub struct Worker {
    clock: IdleClock,
    ledger: DropLedger,
    dispatcher: Dispatcher,
    sink: Box<dyn PlaySink + Send>,
    earcons: EarconPlayer,
    detector: Detector,
    config: OrganConfig,
    /// QQ4 soak instrument (R-C/R-D). None in tests that assert counting
    /// elsewhere; the daemon always attaches one.
    soak: Option<Arc<crate::soak::SoakShared>>,
}

impl Worker {
    pub fn new(
        config: OrganConfig,
        lanes: Vec<Box<dyn RenderLane + Send>>,
        sink: Box<dyn PlaySink + Send>,
        ledger_path: Option<PathBuf>,
        breaker: BreakerConfig,
    ) -> Self {
        Worker {
            clock: IdleClock::new(),
            ledger: DropLedger::new(ledger_path),
            dispatcher: Dispatcher::new(lanes, "v1").with_breaker(breaker),
            sink,
            earcons: EarconPlayer::default(),
            detector: Detector::new(DetectOptions::default()),
            config,
            soak: None,
        }
    }

    /// Attach the soak instrument (the daemon wires the same Arc into
    /// `/health` — one home, many writers).
    pub fn with_soak(mut self, soak: Arc<crate::soak::SoakShared>) -> Self {
        self.soak = Some(soak);
        self
    }

    /// Pop the next due item off the front stage (refreshing the busy
    /// echo first). Callers hold the front lock ONLY for this call.
    fn next_due(&mut self, front: &mut FrontStage, now_s: f64) -> Option<(SayItem, SpeechPath)> {
        front.busy_echo = self.clock.busy_now(now_s);
        let item = front.queue.pop(now_s, &self.clock, &mut self.ledger)?;
        let path = front
            .paths
            .remove(&item.seq)
            .unwrap_or(SpeechPath::GeneralSpeech);
        Some((item, path))
    }

    /// Speak one item through the full route: detect → resolve (R-B) →
    /// dispatch → chime table. NO front lock is held (speech takes
    /// seconds); the counts ride back in a [`Delta`].
    fn speak_item(&mut self, item: SayItem, path: SpeechPath, now_s: f64, now_ms: u128) -> Delta {
        let mut d = Delta::default();
        let entry: LabelConfig = self
            .config
            .labels
            .get(&item.label)
            .cloned()
            .unwrap_or_default();
        let utt = self.detector.detect(&item.text, entry.declared);
        if let Some(s) = &self.soak {
            s.record_detect(&utt);
        }
        let lang = majority_lang(&utt);
        match voiceset::resolve(&entry.set, lang, path, &self.config.registry) {
            RouteDecision::Speak(voice) => {
                if let Some(spec) = self.config.registry.voice(&voice).cloned() {
                    self.dispatch_and_chime(&item, &spec, now_s, now_ms, &mut d);
                } else {
                    // resolve() already checked admission; a None here is
                    // a registry/config inconsistency — degrade loudly.
                    self.honest_degrade(&item, now_s, &mut d);
                }
            }
            RouteDecision::Substitute { voice, warning } => {
                if let Some(spec) = self.config.registry.voice(&voice).cloned() {
                    let before = d.spoken;
                    self.dispatch_and_chime(&item, &spec, now_s, now_ms, &mut d);
                    if warning && d.spoken > before {
                        d.substituted += 1;
                        if self
                            .earcons
                            .play("substitute", &mut self.clock, now_s, &mut *self.sink)
                        {
                            d.earcons += 1;
                        }
                    }
                } else {
                    self.honest_degrade(&item, now_s, &mut d);
                }
            }
            RouteDecision::Degrade { .. } => self.honest_degrade(&item, now_s, &mut d),
        }
        d
    }

    /// Dispatcher speak + the loss side of the chime table.
    fn dispatch_and_chime(
        &mut self,
        item: &SayItem,
        spec: &crate::registry::VoiceSpec,
        now_s: f64,
        now_ms: u128,
        d: &mut Delta,
    ) {
        let t0 = Instant::now();
        let outcome = self.dispatcher.speak(
            item,
            spec,
            1.0,
            now_s,
            now_ms,
            &mut self.clock,
            &mut self.ledger,
            &mut *self.sink,
        );
        let ms = t0.elapsed().as_millis() as u64;
        if let Some(s) = &self.soak {
            match &outcome {
                SayOutcome::Spoke { cache_hit } => {
                    s.record_say(&spec.generator, true, *cache_hit, ms)
                }
                SayOutcome::Dropped { .. } => s.record_say(&spec.generator, false, false, ms),
            }
        }
        match outcome {
            SayOutcome::Spoke { cache_hit } => {
                d.spoken += 1;
                if cache_hit {
                    d.cache_hits += 1;
                }
            }
            SayOutcome::Dropped { reason, anomaly } => {
                d.dropped += 1;
                // The audio path is alive when the RENDER lane lost the
                // line — the daemon degraded to a chime here; so does the
                // organ, with the fail motif (GA3 trips and render
                // errors; process_error means the sink is dead — no
                // chime, the ledger row is the truth).
                if (anomaly || reason.starts_with("render:"))
                    && self
                        .earcons
                        .play("bee.fail", &mut self.clock, now_s, &mut *self.sink)
                {
                    d.earcons += 1;
                }
            }
        }
    }

    /// R-B honest degrade: nothing is spoken; the quiet chime fires, the
    /// loss is ledgered LOSSY (the operator did not hear what he should
    /// have), the count is visible.
    fn honest_degrade(&mut self, item: &SayItem, now_s: f64, d: &mut Delta) {
        if let Some(s) = &self.soak {
            s.record_degrade();
        }
        self.ledger.record(item, "render_error", now_s);
        d.degraded += 1;
        d.dropped += 1;
        if self
            .earcons
            .play("degrade", &mut self.clock, now_s, &mut *self.sink)
        {
            d.earcons += 1;
        }
    }

    /// Caller-fired life event (attention/done/...). Returns whether the
    /// chime actually played (counted by the caller).
    pub fn play_earcon(&mut self, event: &str, now_s: f64) -> bool {
        self.earcons
            .play(event, &mut self.clock, now_s, &mut *self.sink)
    }

    pub fn ledger_health(&self) -> DropHealth {
        self.ledger.health()
    }

    pub fn cache_stats(&self) -> crate::gramophone::CacheStats {
        self.dispatcher.cache_stats()
    }

    /// Stand-down: every queued-but-unspoken item passes the ledger (the
    /// only legal way to empty the queue — orphaned accounting is how
    /// losses hide).
    fn stand_down(&mut self, front: &mut FrontStage, now_s: f64) -> usize {
        front.busy_echo = self.clock.busy_now(now_s);
        let n = front
            .queue
            .drop_pending("stand_down", now_s, &mut self.ledger);
        front.paths.clear();
        front.counts.queue_len = 0;
        n
    }
}

/// Drain every due item: pop under the front lock, speak without it,
/// fold counts back under the lock. Deterministic given (now_s, now_ms).
fn drain(worker: &mut Worker, front: &Mutex<FrontStage>, now_s: f64, now_ms: u128) -> u32 {
    let mut spoken = 0;
    loop {
        let due = {
            let Ok(mut f) = front.lock() else { return spoken };
            worker.next_due(&mut f, now_s)
        };
        let Some((item, path)) = due else { return spoken };
        let delta = worker.speak_item(item, path, now_s, now_ms);
        spoken += delta.spoken + delta.dropped;
        if let Ok(mut f) = front.lock() {
            delta.apply(&mut f);
        }
    }
}

/// The language an utterance speaks: segment majority; ties and empty
/// verdicts break LT (the F-A1 ambiguity law — uncertain LT/EN routes
/// LT: Leonas reading EN is accented-but-correct; ryan reading LT is the
/// banned garbage).
fn majority_lang(utt: &Utterance) -> Lang {
    let mut lt = 0usize;
    let mut en = 0usize;
    for s in &utt.segments {
        match s.decision.lang {
            Lang::Lt => lt += 1,
            Lang::En => en += 1,
        }
    }
    if lt >= en {
        Lang::Lt
    } else {
        Lang::En
    }
}

// ---------------------------------------------------------------------------
// SayService — the thread shell the httpd talks to
// ---------------------------------------------------------------------------

enum Msg {
    /// A life-event chime (attention/done/...), played between speeches.
    Earcon(String),
    /// Drain now (a fresh submission wakes the worker past the poll).
    Wake,
    Quit,
}

/// The running say service: `say()` admits from any thread (brief lock),
/// the worker thread speaks; `earcon()` fires a life-event chime.
pub struct SayService {
    front: Arc<Mutex<FrontStage>>,
    tx: Sender<Msg>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

impl SayService {
    /// Assemble + start. `lanes`/`sink` are caller-assembled so the bin
    /// (P4) owns real adapters while tests stub them; the worker thread
    /// takes ownership of the speaking machinery.
    pub fn start(
        config: OrganConfig,
        lanes: Vec<Box<dyn RenderLane + Send>>,
        sink: Box<dyn PlaySink + Send>,
        ledger_path: Option<PathBuf>,
        breaker: BreakerConfig,
        soak: Option<Arc<crate::soak::SoakShared>>,
    ) -> SayService {
        let front = Arc::new(Mutex::new(FrontStage::new()));
        let (tx, rx): (Sender<Msg>, Receiver<Msg>) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let mut worker = Worker::new(config, lanes, sink, ledger_path, breaker);
        if let Some(s) = soak {
            worker = worker.with_soak(s);
        }
        let front_w = front.clone();
        let stop_w = stop.clone();
        let handle = std::thread::Builder::new()
            .name("caddis-voice-say".into())
            .spawn(move || {
                let t0 = Instant::now();
                loop {
                    let now_s = wall_s();
                    let now_ms = t0.elapsed().as_millis();
                    drain(&mut worker, &front_w, now_s, now_ms);
                    if stop_w.load(Ordering::Relaxed) {
                        break;
                    }
                    match rx.recv_timeout(WORKER_POLL) {
                        Ok(Msg::Earcon(ev)) => {
                            if worker.play_earcon(&ev, wall_s())
                                && front_w.lock().map(|mut f| f.counts.earcons_played += 1).is_err()
                            {
                                // Counting is best-effort; the chime played.
                            }
                        }
                        Ok(Msg::Wake) => {}
                        Ok(Msg::Quit) => break,
                        Err(mpsc::RecvTimeoutError::Timeout) => {}
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                }
                // Clean close: drain what is due, then ledger the rest.
                drain(&mut worker, &front_w, wall_s(), t0.elapsed().as_millis());
                if let Ok(mut f) = front_w.lock() {
                    worker.stand_down(&mut f, wall_s());
                }
            })
            .expect("say worker thread spawns");
        SayService {
            front,
            tx,
            stop,
            handle: Some(handle),
        }
    }

    /// Admit one line. Returns the admission verdict + queue depth.
    pub fn say(
        &self,
        label: &str,
        text: &str,
        narration: bool,
        priority: u8,
        path: SpeechPath,
    ) -> (Admission, usize) {
        let now_s = wall_s();
        let mut f = self.front.lock().expect("front lock");
        let adm = submit_stage(&mut f, label, text, narration, priority, path, now_s);
        let depth = f.queue.len();
        let _ = self.tx.send(Msg::Wake);
        (adm, depth)
    }

    /// Fire a life-event chime (attention/done/...). Refuses unknown
    /// events against the embedded set (caller mistakes are not chimes).
    pub fn earcon(&self, event: &str) -> Result<(), String> {
        if !EarconPlayer::knows(event) {
            return Err(format!("unknown earcon event {event:?}"));
        }
        self.tx
            .send(Msg::Earcon(event.to_string()))
            .map_err(|_| "say worker gone".to_string())
    }

    pub fn counts(&self) -> SayCounts {
        self.front
            .lock()
            .map(|f| f.counts.clone())
            .unwrap_or_default()
    }

    /// Stop the worker: final drain, stand-down ledgering, join.
    pub fn stop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Msg::Quit);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

impl Drop for SayService {
    fn drop(&mut self) {
        self.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::{AdapterErr, RenderedAudio};
    use crate::registry::VoiceSpec;
    use crate::voiceset::VoiceSet;
    use std::cell::Cell;

    // ---- fakes ------------------------------------------------------------

    struct Lane {
        name: &'static str,
        fail: bool,
        renders: Cell<u32>,
    }
    impl RenderLane for Lane {
        fn generator(&self) -> &str {
            self.name
        }
        fn render(&self, _v: &VoiceSpec, _t: &str, _ls: f64) -> Result<RenderedAudio, AdapterErr> {
            self.renders.set(self.renders.get() + 1);
            if self.fail {
                return Err(AdapterErr("stub down".into()));
            }
            Ok(RenderedAudio {
                bytes: vec![1, 2, 3, 4],
                format: crate::adapter::AudioFormat::Wav,
                generator: self.name.into(),
                voice: "v".into(),
                elapsed_ms: 1,
                cap_ms: 1500,
                over_cap: false,
            })
        }
    }
    /// A shareable counting sink: the worker owns the Box, tests read
    /// the Arc from the test thread (atomic — the worker thread mutates
    /// it while we read; Cell is Send but not Sync and would poison the
    /// thread spawn).
    struct SinkInner {
        plays: std::sync::atomic::AtomicU32,
        wavs: Mutex<Vec<usize>>,
    }
    struct Sink {
        inner: Arc<SinkInner>,
    }
    impl PlaySink for Sink {
        fn play(&mut self, wav: &[u8]) -> bool {
            use std::sync::atomic::Ordering;
            self.inner.plays.fetch_add(1, Ordering::Relaxed);
            if let Ok(mut w) = self.inner.wavs.lock() {
                w.push(wav.len());
            }
            true
        }
    }
    fn new_sink() -> (Sink, Arc<SinkInner>) {
        let inner = Arc::new(SinkInner {
            plays: std::sync::atomic::AtomicU32::new(0),
            wavs: Mutex::new(Vec::new()),
        });
        (
            Sink {
                inner: inner.clone(),
            },
            inner,
        )
    }

    fn breaker_small() -> BreakerConfig {
        BreakerConfig {
            capacity: 1,
            refill_per_min: 0,
            cooldown_ms: 60_000,
        }
    }

    fn big_breaker() -> BreakerConfig {
        BreakerConfig {
            capacity: 100,
            refill_per_min: 100,
            cooldown_ms: 1_000,
        }
    }

    fn lanes_both() -> Vec<Box<dyn RenderLane + Send>> {
        vec![
            Box::new(Lane {
                name: "piper",
                fail: false,
                renders: Cell::new(0),
            }),
            Box::new(Lane {
                name: "leonas",
                fail: false,
                renders: Cell::new(0),
            }),
        ]
    }

    // ---- worker core -------------------------------------------------------

    #[test]
    fn end_to_end_speak_coalesce_and_counts() {
        let (sink, inner) = new_sink();
        let mut w = Worker::new(
            OrganConfig::default(),
            lanes_both(),
            Box::new(sink),
            None,
            big_breaker(),
        );
        let front = Mutex::new(FrontStage::new());
        {
            let mut f = front.lock().unwrap();
            assert!(matches!(
                submit_stage(&mut f, "sergeant", "Labas rytas, operatoriau.", true, 1, SpeechPath::GeneralSpeech, 10.0),
                Admission::Queued
            ));
            // Same line inside the window: coalesced (non-critical).
            assert!(matches!(
                submit_stage(&mut f, "sergeant", "Labas rytas, operatoriau.", true, 1, SpeechPath::GeneralSpeech, 10.5),
                Admission::Coalesced
            ));
        }
        let spoke = drain(&mut w, &front, 12.0, 12_000);
        assert_eq!(spoke, 1, "one line admitted of the burst");
        let c = front.lock().unwrap().counts.clone();
        assert_eq!((c.submitted, c.coalesced, c.spoken), (2, 1, 1));
        // LT diacritic text on the sergeant set: the exact LT voice —
        // no substitution, no chime.
        assert_eq!(c.substituted, 0);
        assert_eq!(c.earcons_played, 0);
        assert_eq!(inner.plays.load(std::sync::atomic::Ordering::Relaxed), 1);
    }

    #[test]
    fn r_b_substitute_fires_warning_and_degrade_fires_chime() {
        let (sink, inner) = new_sink();
        let mut cfg = OrganConfig::default();
        cfg.labels.insert(
            "enonly".into(),
            LabelConfig {
                declared: None,
                set: VoiceSet {
                    lt: None,
                    en: Some("en_US-ryan".into()),
                },
            },
        );
        let mut w = Worker::new(cfg, lanes_both(), Box::new(sink), None, big_breaker());
        let front = Mutex::new(FrontStage::new());
        {
            let mut f = front.lock().unwrap();
            // LT text, label with NO LT voice: GeneralSpeech substitutes
            // the first admitted LT voice (LeonasNeural) + warning earcon.
            submit_stage(&mut f, "enonly", "Labas, čia substitucija.", true, 1, SpeechPath::GeneralSpeech, 5.0);
            // Same label on the confirm path: honest degrade (chime + ledger).
            submit_stage(&mut f, "enonly", "Patvirtinta.", true, 1, SpeechPath::GatedConfirm, 6.0);
        }
        drain(&mut w, &front, 8.0, 8_000);
        let c = front.lock().unwrap().counts.clone();
        assert_eq!(c.substituted, 1, "general path substituted");
        assert_eq!(c.degraded, 1, "confirm path degraded");
        assert_eq!(c.spoken, 1, "only the substituted line spoke");
        // Two chimes + one speech = three sink plays.
        assert_eq!(inner.plays.load(std::sync::atomic::Ordering::Relaxed), 3);
        let h = w.ledger_health();
        assert_eq!(h.by_reason.get("render_error"), Some(&1), "degrade is lossy");
        assert!(h.undelivered >= 1);
    }

    #[test]
    fn ga3_trip_fires_fail_chime() {
        let (sink, inner) = new_sink();
        let mut w = Worker::new(
            OrganConfig::default(),
            vec![Box::new(Lane {
                name: "piper",
                fail: false,
                renders: Cell::new(0),
            })],
            Box::new(sink),
            None,
            breaker_small(),
        );
        let front = Mutex::new(FrontStage::new());
        {
            let mut f = front.lock().unwrap();
            submit_stage(&mut f, "sergeant", "Hello operator.", true, 1, SpeechPath::GeneralSpeech, 1.0);
            submit_stage(&mut f, "sergeant", "Different line entirely.", true, 1, SpeechPath::GeneralSpeech, 1.1);
        }
        drain(&mut w, &front, 2.0, 2_000);
        let c = front.lock().unwrap().counts.clone();
        assert_eq!(c.spoken, 1);
        assert_eq!(c.dropped, 1, "second render tripped the capacity-1 breaker");
        assert_eq!(c.earcons_played, 1, "the fail chime fired");
        assert_eq!(inner.plays.load(std::sync::atomic::Ordering::Relaxed), 2, "one speech + one chime");
    }

    #[test]
    fn stand_down_ledgers_pending() {
        let (sink, _inner) = new_sink();
        let mut w = Worker::new(OrganConfig::default(), lanes_both(), Box::new(sink), None, big_breaker());
        let front = Mutex::new(FrontStage::new());
        {
            let mut f = front.lock().unwrap();
            submit_stage(&mut f, "sergeant", "Nebaigta eilutė.", true, 1, SpeechPath::GeneralSpeech, 1.0);
        }
        let n = {
            let mut f = front.lock().unwrap();
            w.stand_down(&mut f, 2.0)
        };
        assert_eq!(n, 1);
        assert_eq!(w.ledger_health().by_reason.get("stand_down"), Some(&1));
        assert_eq!(front.lock().unwrap().counts.queue_len, 0);
    }

    #[test]
    fn majority_lang_ties_break_lt() {
        let mut d = Detector::new(DetectOptions::default());
        let u = d.detect("Hello labas", None);
        assert_eq!(majority_lang(&u), Lang::Lt);
    }

    // ---- service shell (thread) -------------------------------------------

    #[test]
    fn service_say_drains_and_stops_clean() {
        let (sink, inner) = new_sink();
        let mut svc = SayService::start(
            OrganConfig::default(),
            lanes_both(),
            Box::new(sink),
            None,
            big_breaker(),
            None,
        );
        let (adm, depth) = svc.say(
            "sergeant",
            "Service smoke line.",
            true,
            0,
            SpeechPath::GeneralSpeech,
        );
        assert!(matches!(adm, Admission::Queued));
        assert_eq!(depth, 1);
        // The worker drains within a wake or two.
        let deadline = Instant::now() + Duration::from_secs(5);
        while Instant::now() < deadline && svc.counts().spoken < 1 {
            std::thread::sleep(Duration::from_millis(25));
        }
        assert!(svc.counts().spoken >= 1, "worker spoke the line");
        assert!(inner.plays.load(std::sync::atomic::Ordering::Relaxed) >= 1);
        assert!(svc.earcon("attention").is_ok());
        assert!(svc.earcon("no.such.event").is_err());
        svc.stop();
    }

    #[test]
    fn soak_records_lane_outcomes_and_detection() {
        let (sink, _inner) = new_sink();
        let soak = crate::soak::shared(None);
        let mut w = Worker::new(
            OrganConfig::default(),
            lanes_both(),
            Box::new(sink),
            None,
            big_breaker(),
        )
        .with_soak(soak.clone());
        let front = Mutex::new(FrontStage::new());
        {
            let mut f = front.lock().unwrap();
            submit_stage(&mut f, "sergeant", "Labas, operatoriau.", true, 1, SpeechPath::GeneralSpeech, 10.0);
            submit_stage(&mut f, "unknown-label", "Patvirtinta.", true, 1, SpeechPath::GatedConfirm, 11.0);
        }
        drain(&mut w, &front, 12.0, 12_000);
        let snap = soak.snapshot();
        // Line 1 (sergeant label, LT diacritics, general path): spoke on
        // the LT lane (embedded registry routes sergeant LT to leonas).
        let leonas = snap
            .lanes
            .iter()
            .find(|(l, _)| l == "leonas")
            .expect("leonas lane");
        assert_eq!((leonas.1.attempts, leonas.1.spoke), (1, 1));
        // Line 2 (UNKNOWN label → empty set → R-B confirm path): honest
        // degrade — route health, recorded under `_route`, never on a
        // render lane.
        let route = snap
            .lanes
            .iter()
            .find(|(l, _)| l == crate::soak::ROUTE_LANE)
            .expect("route lane");
        assert_eq!(
            (route.1.attempts, route.1.dropped, route.1.degraded),
            (1, 1, 1)
        );
        // Detection telemetry: both lines detected, both cache misses;
        // "Labas, operatoriau." is UNMARKED LT → the L2 trigram decides.
        assert!(snap.detect.l2_trigram >= 1);
        assert_eq!(snap.detect.cache_miss, 2);
        let spoke_total: u64 = snap.lanes.iter().map(|(_, c)| c.spoke).sum();
        assert_eq!(spoke_total, 1, "one spoke, one degraded");
    }
}
