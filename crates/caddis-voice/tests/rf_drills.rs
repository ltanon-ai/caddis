//! Integration: the T-35 R-F in-situ drill script, slice 1 — failure
//! drills against the ASSEMBLED gramophone (queue → dispatch → ledger →
//! earcon table → play sink) with FAULT lanes wired in place of the
//! real generators.
//!
//! The R-F list is the quorum-approved P3/P4 acceptance gate
//! (state/briefs/t35-voice-amendment-quorum/VERDICT.md, F-A6 base +
//! amendments). This file covers the render-lane failure family:
//!
//! - Drill 1  Leonas-down — LT speech whose primary lane fails at render
//! - Drill 1 Leonas-down — LT speech whose primary lane fails at render drops with a `render_error` ledger row and the `bee.fail` chime; no other lane renders (never a wrong-voice render).
//! - Drill 4 mid-utterance generator death — the generator dies after a successful utterance; the next utterance through it drops with a ledger row, and the queue keeps working through a surviving lane.
//! - Drill 5 dual-LT cascade — Leonas AND Ona lanes both down; LT speech on BOTH paths (general + gated confirm) drops with ledger rows and fail chimes; panel counters update inside the same drain window.
//!
//!
//! What stays OUT of this file (per the verdict): audio TRUTH stays with
//! the operator's ear; the socket layer's happy path is covered by
//! gramophone_e2e.rs; drills 2/3/6/7 (spawn-failure recovery, cache
//! corruption, slow-lane deadline, GA2 injection reject) live at their
//! owning units (adapter.rs, play.rs, edgetts.rs) and get their in-situ
//! pass in the next slice.

use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use caddis_voice::adapter::{AdapterErr, AudioFormat, RenderedAudio};
use caddis_voice::config::OrganConfig;
use caddis_voice::registry::VoiceSpec;
use caddis_voice::say::{PlaySink, RenderLane};
use caddis_voice::sayd::SayService;
use caddis_voice::{Admission, BreakerConfig, SpeechPath};

// ---------------------------------------------------------------------------
// Fault probes — the drill injection points
// ---------------------------------------------------------------------------

struct FaultInner {
    name: &'static str,
    fail: AtomicBool,
    /// Custom failure text (deadline-shaped verdicts etc.); None →
    /// the default "lane down" line.
    fail_msg: std::sync::Mutex<Option<String>>,
    renders: AtomicU32,
}

impl FaultProbe {
    /// Fail with a custom error shape (e.g. an R-D deadline verdict).
    /// (std-only crate: no parking_lot — poison-safe into_inner instead
    /// of unwrap, per the rs-parking-lot rule's target pattern.)
    fn fail_with(&self, msg: &str) {
        let mut g = self.0.fail_msg.lock().unwrap_or_else(|e| e.into_inner());
        *g = Some(msg.to_string());
        self.0.fail.store(true, Ordering::SeqCst);
    }
}

/// A cloneable handle to one render lane; cloning keeps the counters
/// shared with the copy boxed into the service.
#[derive(Clone)]
struct FaultProbe(Arc<FaultInner>);

impl FaultProbe {
    fn healthy(name: &'static str) -> Self {
        Self(Arc::new(FaultInner {
            name,
            fail: AtomicBool::new(false),
            fail_msg: std::sync::Mutex::new(None),
            renders: AtomicU32::new(0),
        }))
    }

    fn down(name: &'static str) -> Self {
        Self(Arc::new(FaultInner {
            name,
            fail: AtomicBool::new(true),
            fail_msg: std::sync::Mutex::new(None),
            renders: AtomicU32::new(0),
        }))
    }

    /// Kill (or revive) the lane mid-queue.
    fn set_fail(&self, on: bool) {
        self.0.fail.store(on, Ordering::SeqCst);
    }

    fn renders(&self) -> u32 {
        self.0.renders.load(Ordering::SeqCst)
    }
}

impl RenderLane for FaultProbe {
    fn generator(&self) -> &str {
        self.0.name
    }

    fn render(
        &self,
        _voice: &VoiceSpec,
        _text: &str,
        _length_scale: f64,
    ) -> Result<RenderedAudio, AdapterErr> {
        self.0.renders.fetch_add(1, Ordering::SeqCst);
        if self.0.fail.load(Ordering::SeqCst) {
            let msg = self
                .0
                .fail_msg
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .clone()
                .unwrap_or_else(|| format!("{}: lane down (drill injection)", self.0.name));
            return Err(AdapterErr(msg));
        }
        Ok(RenderedAudio {
            bytes: wav_blob(),
            format: AudioFormat::Wav,
            generator: self.0.name.to_string(),
            voice: "drill-probe".into(),
            elapsed_ms: 3,
            cap_ms: 2500,
            over_cap: false,
        })
    }
}

/// A minimal VALID RIFF/WAVE mono blob (same header shape the e2e
/// SilentLane emits) — the sink is fake; audio truth is the operator's ear.
fn wav_blob() -> Vec<u8> {
    let rate = 22_050u32;
    let frames = 2205usize; // 0.1 s mono
    let mut b = Vec::new();
    b.extend_from_slice(b"RIFF");
    let data_len = (frames * 2) as u32;
    b.extend_from_slice(&(36 + data_len).to_le_bytes());
    b.extend_from_slice(b"WAVEfmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&data_len.to_le_bytes());
    b.extend(std::iter::repeat_n([0u8, 0], frames).flatten());
    b
}

/// The counting play sink (earcon WAVs arrive here too).
struct RecSink {
    plays: AtomicU32,
}

impl PlaySink for RecSink {
    fn play(&mut self, _wav: &[u8]) -> bool {
        self.plays.fetch_add(1, Ordering::SeqCst);
        true
    }
}

fn breaker_big() -> BreakerConfig {
    BreakerConfig {
        capacity: 100,
        refill_per_min: 100,
        cooldown_ms: 60_000,
    }
}

/// A unique ledger directory per drill (cleaned up best-effort).
fn drill_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("caddis-rf-drill-{}-{}", std::process::id(), tag));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("drill dir");
    dir
}

fn start_svc(dir: &std::path::Path, lanes: Vec<FaultProbe>) -> SayService {
    let dyn_lanes: Vec<Box<dyn RenderLane + Send>> = lanes
        .into_iter()
        .map(|l| Box::new(l) as Box<dyn RenderLane + Send>)
        .collect();
    SayService::start(
        OrganConfig::default(),
        dyn_lanes,
        Box::new(RecSink {
            plays: AtomicU32::new(0),
        }),
        Some(dir.join("drops.jsonl")),
        breaker_big(),
        None,
    )
}

/// Poll until `cond` holds or the deadline passes; on timeout, panic with
/// the counts snapshot so the failure names the drill leg that stalled.
fn wait_for(what: &str, svc: &SayService, mut cond: impl FnMut(&SayService) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !cond(svc) {
        if Instant::now() > deadline {
            panic!(
                "drill stalled waiting for {what}; counts = {:?}",
                svc.counts()
            );
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

/// Count ledger rows carrying `needle` across the drill's JSONL files.
fn ledger_rows_with(dir: &std::path::Path, needle: &str) -> usize {
    let mut n = 0;
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            let p = e.path();
            if p.extension().and_then(|x| x.to_str()) == Some("jsonl") {
                if let Ok(body) = std::fs::read_to_string(&p) {
                    n += body.lines().filter(|l| l.contains(needle)).count();
                }
            }
        }
    }
    n
}

fn say_queued(svc: &SayService, label: &str, text: &str, path: SpeechPath) {
    let (adm, _) = svc.say(label, text, false, 0, path);
    assert!(
        matches!(adm, Admission::Queued),
        "admission refused for {label:?}: {adm:?}"
    );
}

/// Drill 1 — Leonas-down: a render-lane failure on LT general speech must
/// fall to the honest degrade ladder (ledger row + `bee.fail` chime),
/// never a wrong-voice render through another lane.
#[test]
fn drill_1_leonas_down_honest_drop() {
    let dir = drill_dir("leonas-down");
    let leonas = FaultProbe::down("leonas");
    let ona = FaultProbe::healthy("ona");
    let piper = FaultProbe::healthy("piper");
    let svc = start_svc(&dir, vec![leonas.clone(), ona.clone(), piper.clone()]);

    say_queued(
        &svc,
        "sergeant",
        "Vienas du trys drill vienas",
        SpeechPath::GeneralSpeech,
    );
    wait_for("leonas-down drop", &svc, |s| s.counts().dropped >= 1);

    let c = svc.counts();
    assert_eq!(c.spoken, 0, "nothing may render wrong-voice");
    assert_eq!(c.degraded, 0, "dispatch drop, not an R-B resolve degrade");
    assert!(c.earcons_played >= 1, "bee.fail chime must fire");
    assert_eq!(leonas.renders(), 1, "Leonas lane tried exactly once");
    assert_eq!(ona.renders(), 0, "no wrong-voice Ona render");
    assert_eq!(piper.renders(), 0, "no wrong-language piper render");
    assert!(
        ledger_rows_with(&dir, "render_error") >= 1,
        "ledger row recorded"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Drill 4 — mid-queue generator death: the piper generator dies after a
/// successful utterance; the next EN utterance drops with a ledger row,
/// and the queue keeps working through a surviving LT lane.
#[test]
fn drill_4_generator_death_queue_survives() {
    let dir = drill_dir("generator-death");
    let piper = FaultProbe::healthy("piper");
    let leonas = FaultProbe::healthy("leonas");
    let ona = FaultProbe::healthy("ona");
    let svc = start_svc(&dir, vec![piper.clone(), leonas.clone(), ona.clone()]);

    // Utterance 1 — piper healthy, speaks.
    say_queued(
        &svc,
        "sergeant",
        "drill four first words",
        SpeechPath::GeneralSpeech,
    );
    wait_for("first speech", &svc, |s| s.counts().spoken >= 1);

    // The generator dies mid-queue.
    piper.set_fail(true);

    // Utterance 2 — same generator: honest drop + ledger row.
    say_queued(
        &svc,
        "sergeant",
        "drill four second words",
        SpeechPath::GeneralSpeech,
    );
    wait_for("post-death drop", &svc, |s| s.counts().dropped >= 1);
    assert!(
        ledger_rows_with(&dir, "render_error") >= 1,
        "ledger row recorded"
    );

    // Utterance 3 — a surviving lane still speaks: the queue is not poisoned.
    say_queued(
        &svc,
        "sergeant",
        "drill keturi po mirties",
        SpeechPath::GeneralSpeech,
    );
    wait_for("surviving-lane speech", &svc, |s| s.counts().spoken >= 2);

    let c = svc.counts();
    assert_eq!(c.spoken, 2, "1 before death + 1 through the surviving lane");
    assert_eq!(c.dropped, 1);

    let _ = std::fs::remove_dir_all(&dir);
}

/// Drill 5 — dual-LT cascade: Leonas AND Ona both down. LT speech on the
/// general path AND on the gated confirm path drops with ledger rows and
/// fail chimes; the panel counters reflect both inside the same window.
#[test]
fn drill_5_dual_lt_cascade() {
    let dir = drill_dir("dual-lt-cascade");
    let leonas = FaultProbe::down("leonas");
    let ona = FaultProbe::down("ona");
    let piper = FaultProbe::healthy("piper");
    let svc = start_svc(&dir, vec![leonas.clone(), ona.clone(), piper.clone()]);

    // General path — sergeant LT (Leonas).
    say_queued(
        &svc,
        "sergeant",
        "drill penki bendrasis kelias",
        SpeechPath::GeneralSpeech,
    );
    // Gated confirm path — kamane LT (Ona): integrity path, honest drop.
    say_queued(
        &svc,
        "kamane",
        "drill patvirtinimo langas",
        SpeechPath::GatedConfirm,
    );

    wait_for("both LT drops", &svc, |s| s.counts().dropped >= 2);

    let c = svc.counts();
    assert_eq!(c.spoken, 0, "no wrong-voice / wrong-language fallback");
    assert_eq!(c.dropped, 2, "both paths dropped");
    assert!(c.earcons_played >= 2, "fail chime on both drops");
    assert!(
        ledger_rows_with(&dir, "render_error") >= 2,
        "both ledger rows recorded"
    );

    let _ = std::fs::remove_dir_all(&dir);
}

/// Drill 2 — VRAM contention (spawn-failure shape): the offline
/// generator cannot start while the GPU is held (child spawn fails).
/// The utterance drops honestly (ledger row + bee.fail chime), the
/// breaker does NOT trip (render error, not a rate anomaly), and the
/// moment contention clears the SAME lane speaks again — recovery
/// without restarting anything.
#[test]
fn drill_2_vram_contention_recovery() {
    let dir = drill_dir("vram-contention");
    let piper = FaultProbe::healthy("piper");
    let leonas = FaultProbe::healthy("leonas");
    let ona = FaultProbe::healthy("ona");
    let svc = start_svc(&dir, vec![piper.clone(), leonas.clone(), ona.clone()]);

    // Utterance 1 — healthy, speaks (EN → piper).
    say_queued(
        &svc,
        "sergeant",
        "drill two healthy words",
        SpeechPath::GeneralSpeech,
    );
    wait_for("pre-contention speech", &svc, |s| s.counts().spoken >= 1);

    // The STT horn takes the GPU: spawn fails (VRAM contention shape).
    piper.fail_with("piper: spawn failed: os error 0 (vram contention drill)");
    say_queued(
        &svc,
        "sergeant",
        "drill two contention words",
        SpeechPath::GeneralSpeech,
    );
    wait_for("contention drop", &svc, |s| s.counts().dropped >= 1);

    let mid = svc.counts();
    assert_eq!(mid.spoken, 1);
    assert_eq!(mid.dropped, 1);
    assert!(
        mid.earcons_played >= 1,
        "bee.fail chime on the spawn failure"
    );
    assert!(
        ledger_rows_with(&dir, "render_error") >= 1,
        "ledger row recorded"
    );
    assert_eq!(
        ledger_rows_with(&dir, "ga3"),
        0,
        "breaker never trips on a spawn failure"
    );

    // Contention clears — the SAME lane recovers on the next utterance.
    piper.set_fail(false);
    say_queued(
        &svc,
        "sergeant",
        "drill two recovered words",
        SpeechPath::GeneralSpeech,
    );
    wait_for("post-contention recovery", &svc, |s| s.counts().spoken >= 2);

    let c = svc.counts();
    assert_eq!(c.spoken, 2, "same lane spoke before and after contention");
    assert_eq!(c.dropped, 1, "no additional drops during recovery");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Drill 6 — slow-lane deadline exceed: the LT lane stalls past the
/// R-D single-attempt budget. The render verdict is the deadline
/// error itself (no wrong-voice fallback, no sibling detour), the
/// drop lands with the bee.fail warning chime + ledger row, and the
/// queue moves ON — the next utterance speaks (no head-of-line block
/// behind the stalled lane).
#[test]
fn drill_6_deadline_exceed_chimes_and_queue_moves_on() {
    let dir = drill_dir("deadline-exceed");
    let leonas = FaultProbe::healthy("leonas");
    let ona = FaultProbe::healthy("ona");
    let piper = FaultProbe::healthy("piper");
    let svc = start_svc(&dir, vec![leonas.clone(), ona.clone(), piper.clone()]);

    // The LT lane stalls past its budget (LT text → leonas).
    leonas.fail_with("r-d deadline exceeded (2500 ms) on attempt 1");
    say_queued(
        &svc,
        "sergeant",
        "drill šeši lėta linija",
        SpeechPath::GeneralSpeech,
    );
    wait_for("deadline drop", &svc, |s| s.counts().dropped >= 1);

    let mid = svc.counts();
    assert_eq!(mid.spoken, 0, "deadline verdict — no wrong-voice render");
    assert!(
        mid.earcons_played >= 1,
        "bee.fail warning chime on deadline exceed"
    );
    assert!(
        ledger_rows_with(&dir, "render_error") >= 1,
        "ledger row recorded"
    );
    assert_eq!(ona.renders(), 0, "no fallback through the sibling LT lane");
    assert_eq!(piper.renders(), 0, "no wrong-language fallback");

    // The lane answers within budget again — the queue moved on.
    leonas.set_fail(false);
    say_queued(
        &svc,
        "sergeant",
        "drill šeši atsakė vėl",
        SpeechPath::GeneralSpeech,
    );
    wait_for("post-deadline speech", &svc, |s| s.counts().spoken >= 1);

    let c = svc.counts();
    assert_eq!(c.spoken, 1);
    assert_eq!(c.dropped, 1);

    let _ = std::fs::remove_dir_all(&dir);
}
