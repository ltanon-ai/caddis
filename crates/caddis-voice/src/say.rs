//! say.rs — the DISPATCH slice (P3 slice b + c): adapter render through
//! the GA3 breaker, WAV cache, and the killable play child, all wired
//! under the idle-clock bracket.
//!
//! Ports the daemon's proven `_speak` bracket (peluda_voice
//! `scheduler_emit.py`): the worker plays audio SYNCHRONOUSLY — one
//! ~200-char narration blocks it for 12-16 s — and that interval is
//! marked SPEAKING on the idle clock so a message queued behind live
//! speech never "ages" (only SILENT time counts). The bracket is also
//! where a hang cannot hide: the wedge backstop lives on the clock, the
//! render/play deadlines live in the adapters and the play child.
//!
//! Order of operations (the daemon's order, kept):
//! 1. CACHE FIRST — a hit costs no lane token and no render (tts.py
//!    checked the wav path before anything else).
//! 2. On a miss the GA3 breaker gates the GENERATOR's bucket (adapter.rs
//!    — one breaker truth, keyed per generator). A trip is an ANOMALY
//!    the caller MUST surface (T-35 verdict GA3):
//!    [`SayOutcome::Dropped`] carries the flag and the ledger row is
//!    written. The daemon degraded to a chime here; the organ's chime
//!    TABLE (fail on lane loss, `substitute`/`degrade` on the R-B
//!    paths) lives one layer up in `sayd` — the service that owns the
//!    earcon player and the routing decisions fires it.
//! 3. Render → validate (GA2, inside the adapters) → play through the
//!    killable child.
//!
//! Every drop path passes the [`DropLedger`] (a dropped message must be
//! LOUD); every render is cached under the daemon's composite key.

use crate::adapter::{AdapterErr, Breaker, RenderedAudio};
use crate::gramophone::{wav_cache_key, CachedAudio, DropLedger, IdleClock, SayItem, WavCache};
use crate::piper::PiperAdapter;
use crate::play::AudioOut;
use crate::registry::VoiceSpec;

/// One render lane the dispatcher may speak through. Implemented by the
/// organ's adapters; tests stub it (the daemon's engine seam).
pub trait RenderLane {
    fn generator(&self) -> &str;
    fn render(
        &self,
        voice: &VoiceSpec,
        text: &str,
        length_scale: f64,
    ) -> Result<RenderedAudio, AdapterErr>;
}

impl RenderLane for PiperAdapter {
    fn generator(&self) -> &str {
        "piper"
    }
    fn render(
        &self,
        voice: &VoiceSpec,
        text: &str,
        length_scale: f64,
    ) -> Result<RenderedAudio, AdapterErr> {
        PiperAdapter::render(self, voice, text, length_scale)
    }
}

/// The play sink seam (tests fake it; production is the killable play
/// child behind [`AudioOut`]).
pub trait PlaySink {
    fn play(&mut self, wav: &[u8]) -> bool;
}

impl PlaySink for AudioOut {
    fn play(&mut self, wav: &[u8]) -> bool {
        AudioOut::play(self, wav).ok()
    }
}

/// One dispatched say line's terminal outcome.
#[derive(Debug, Clone, PartialEq)]
pub enum SayOutcome {
    /// Spoken end-to-end. `cache_hit` = the render was already in the
    /// WAV cache (the soak story: cost avoided).
    Spoke { cache_hit: bool },
    /// Never heard. `reason` is the ledger row; `anomaly` = GA3 trip
    /// (the audit line the verdict requires the caller to surface).
    Dropped { reason: String, anomaly: bool },
}

/// The dispatch half of the gramophone: lanes + breaker + cache + the
/// play sink, all owned. The worker (sayd) owns this whole struct, like
/// the daemon's worker thread owned the scheduler lock; lanes are OWNED
/// boxes (v1 wires piper; leonas/ona lanes land with P4 — an admitted
/// voice whose generator has no lane drops LOUD with `no lane wired`).
pub struct Dispatcher {
    lanes: Vec<Box<dyn RenderLane + Send>>,
    breaker: Breaker,
    cache: WavCache,
    phrase_pack_version: String,
}

impl Dispatcher {
    pub fn new(lanes: Vec<Box<dyn RenderLane + Send>>, phrase_pack_version: &str) -> Self {
        Dispatcher {
            lanes,
            breaker: Breaker::default(),
            cache: WavCache::new(64 * 1024 * 1024),
            phrase_pack_version: phrase_pack_version.to_string(),
        }
    }

    /// TEST LANE ONLY: inject breaker bounds (a trip must be provable
    /// without burning 12 real narrations).
    pub fn with_breaker(mut self, cfg: crate::adapter::BreakerConfig) -> Self {
        self.breaker = Breaker::new(cfg);
        self
    }

    pub fn cache_stats(&self) -> crate::gramophone::CacheStats {
        self.cache.stats()
    }

    /// The lane that renders for a voice's generator (None = generator
    /// admitted in the registry but no lane wired in this build).
    fn lane_for(&self, generator: &str) -> Option<&(dyn RenderLane + Send)> {
        self.lanes
            .iter()
            .find(|l| l.generator() == generator)
            .map(|l| l.as_ref())
    }

    fn key(&self, item: &SayItem, voice: &VoiceSpec, length_scale: f64) -> String {
        // The engine field is the VOICE's generator — with owned lanes the
        // dispatcher no longer has a single generator identity, and the
        // cache must not collapse two generators' renders of one text.
        // rate/pitch are empty on the piper lane (it shapes speech only
        // through length_scale; the key keeps the daemon's full shape).
        wav_cache_key(
            &item.text,
            &voice.id,
            &voice.generator,
            "",
            "",
            length_scale,
            &self.phrase_pack_version,
        )
    }

    /// Speak one popped queue item: idle-clock bracket around cache →
    /// breaker → render → play, ledger rows on every loss.
    ///
    /// `now_s` is the idle-clock's seconds domain; `now_ms` the breaker's
    /// monotonic ms domain (two clocks, one truth per user). The bracket
    /// CLOSES at `now_s + real elapsed` — the daemon measured real speech
    /// time; a bracket that closed at its own opening time would credit
    /// zero busy seconds and falsely age every cue queued mid-utterance.
    #[allow(clippy::too_many_arguments)]
    pub fn speak(
        &mut self,
        item: &SayItem,
        voice: &VoiceSpec,
        length_scale: f64,
        now_s: f64,
        now_ms: u128,
        clock: &mut IdleClock,
        ledger: &mut DropLedger,
        sink: &mut dyn PlaySink,
    ) -> SayOutcome {
        let key = self.key(item, voice, length_scale);
        clock.start_speaking(now_s);
        let t0 = std::time::Instant::now();
        let outcome = self.dispatch(item, voice, length_scale, &key, now_ms, ledger, sink);
        clock.stop_speaking(now_s + t0.elapsed().as_secs_f64());
        outcome
    }

    #[allow(clippy::too_many_arguments)]
    fn dispatch(
        &mut self,
        item: &SayItem,
        voice: &VoiceSpec,
        length_scale: f64,
        key: &str,
        now_ms: u128,
        ledger: &mut DropLedger,
        sink: &mut dyn PlaySink,
    ) -> SayOutcome {
        // 1. Cache first — no token, no render.
        if let Some(hit) = self.cache.get(key) {
            return self.play(item, &hit.bytes, true, now_s_of(now_ms), ledger, sink);
        }
        // 2. GA3 gates the lane — per GENERATOR (the voice's own bucket).
        if let Err(tripped) = self.breaker.try_acquire(&voice.generator, now_ms) {
            ledger.record(item, "render_error", now_s_of(now_ms));
            return SayOutcome::Dropped {
                reason: format!(
                    "ga3 tripped: {} until {}ms",
                    tripped.generator, tripped.retry_not_before_ms
                ),
                anomaly: tripped.anomaly,
            };
        }
        // 3. Render, cache, play.
        let Some(lane) = self.lane_for(&voice.generator) else {
            ledger.record(item, "render_error", now_s_of(now_ms));
            return SayOutcome::Dropped {
                reason: format!("render: no {} lane wired in this build", voice.generator),
                anomaly: false,
            };
        };
        match lane.render(voice, &item.text, length_scale) {
            Err(e) => {
                ledger.record(item, "render_error", now_s_of(now_ms));
                SayOutcome::Dropped {
                    reason: format!("render: {e}"),
                    anomaly: false,
                }
            }
            Ok(audio) => {
                let bytes = audio.bytes;
                let rendered_ms = audio.elapsed_ms;
                let spoke = self.play(item, &bytes, false, now_s_of(now_ms), ledger, sink);
                // Store AFTER playing: the sink borrows, the cache takes
                // ownership — no clone of a whole utterance.
                self.cache.put(
                    key,
                    CachedAudio {
                        bytes,
                        format: audio.format,
                        rendered_ms,
                    },
                );
                spoke
            }
        }
    }

    fn play(
        &mut self,
        item: &SayItem,
        bytes: &[u8],
        cache_hit: bool,
        now_s: f64,
        ledger: &mut DropLedger,
        sink: &mut dyn PlaySink,
    ) -> SayOutcome {
        if sink.play(bytes) {
            SayOutcome::Spoke { cache_hit }
        } else {
            ledger.record(item, "process_error", now_s);
            SayOutcome::Dropped {
                reason: "play: audio failed".into(),
                anomaly: false,
            }
        }
    }
}

/// The breaker speaks ms; the ledger speaks seconds. One conversion,
/// one place (never two domains drifting apart silently).
fn now_s_of(now_ms: u128) -> f64 {
    now_ms as f64 / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adapter::BreakerConfig;
    use crate::gramophone::DropHealth;
    use crate::lang::Lang;

    struct FakeLane {
        fail: bool,
        renders: std::sync::Arc<std::sync::atomic::AtomicU32>,
        name: &'static str,
    }
    impl FakeLane {
        fn piper(fail: bool) -> (Self, std::sync::Arc<std::sync::atomic::AtomicU32>) {
            Self::named(fail, "piper")
        }
        fn named(
            fail: bool,
            name: &'static str,
        ) -> (Self, std::sync::Arc<std::sync::atomic::AtomicU32>) {
            let renders = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
            (
                FakeLane {
                    fail,
                    renders: renders.clone(),
                    name,
                },
                renders,
            )
        }
    }
    impl RenderLane for FakeLane {
        fn generator(&self) -> &str {
            self.name
        }
        fn render(&self, _v: &VoiceSpec, _t: &str, _ls: f64) -> Result<RenderedAudio, AdapterErr> {
            use std::sync::atomic::Ordering;
            self.renders.fetch_add(1, Ordering::Relaxed);
            if self.fail {
                return Err(AdapterErr("stub lane down".into()));
            }
            Ok(RenderedAudio {
                bytes: vec![1, 2, 3, 4],
                format: crate::adapter::AudioFormat::Wav,
                generator: self.name.into(),
                voice: "en_US-ryan".into(),
                elapsed_ms: 42,
                cap_ms: 1500,
                over_cap: false,
            })
        }
    }

    struct FakeSink {
        fail: bool,
        plays: std::cell::Cell<u32>,
    }
    impl PlaySink for FakeSink {
        fn play(&mut self, _wav: &[u8]) -> bool {
            self.plays.set(self.plays.get() + 1);
            !self.fail
        }
    }

    fn voice() -> VoiceSpec {
        VoiceSpec {
            id: "en_US-ryan".into(),
            generator: "piper".into(),
            lang: Lang::En,
        }
    }

    #[test]
    fn speak_happy_path_then_cache_hit() {
        let (lane, renders) = FakeLane::piper(false);
        let mut sink = FakeSink {
            fail: false,
            plays: std::cell::Cell::new(0),
        };
        let mut clock = IdleClock::new();
        let mut ledger = DropLedger::new(None);
        let mut d = Dispatcher::new(vec![Box::new(lane)], "v1");
        let v = voice();
        let out = d.speak(
            &item("hello"),
            &v,
            1.0,
            10.0,
            10_000,
            &mut clock,
            &mut ledger,
            &mut sink,
        );
        assert_eq!(out, SayOutcome::Spoke { cache_hit: false });
        assert_eq!(renders.load(std::sync::atomic::Ordering::Relaxed), 1);

        // Second identical line: cache hit, no second render.
        let out2 = d.speak(
            &item("hello"),
            &v,
            1.0,
            20.0,
            20_000,
            &mut clock,
            &mut ledger,
            &mut sink,
        );
        assert_eq!(out2, SayOutcome::Spoke { cache_hit: true });
        assert_eq!(
            renders.load(std::sync::atomic::Ordering::Relaxed),
            1,
            "cache hit must not render again"
        );
        let st = d.cache_stats();
        assert_eq!((st.hits, st.misses, st.stores), (1, 1, 1));
    }

    fn item(text: &str) -> SayItem {
        SayItem {
            label: "sergeant".into(),
            text: text.into(),
            narration: true,
            priority: 1,
            seq: 0,
            enqueued_at: 0.0,
            busy_at_enqueue: 0.0,
            due_at: 0.0,
        }
    }

    #[test]
    fn multi_lane_dispatch_routes_by_generator_and_missing_lane_is_loud() {
        let (piper_lane, piper_renders) = FakeLane::piper(false);
        let (leonas_lane, leonas_renders) = FakeLane::named(false, "leonas");
        let mut sink = FakeSink {
            fail: false,
            plays: std::cell::Cell::new(0),
        };
        let mut clock = IdleClock::new();
        let mut ledger = DropLedger::new(None);
        let mut d = Dispatcher::new(vec![Box::new(piper_lane), Box::new(leonas_lane)], "v1");
        // EN voice → the piper lane renders it.
        let en = voice();
        assert!(matches!(
            d.speak(
                &item("hello"),
                &en,
                1.0,
                1.0,
                1_000,
                &mut clock,
                &mut ledger,
                &mut sink
            ),
            SayOutcome::Spoke { .. }
        ));
        assert_eq!(
            (
                piper_renders.load(std::sync::atomic::Ordering::Relaxed),
                leonas_renders.load(std::sync::atomic::Ordering::Relaxed)
            ),
            (1, 0)
        );
        // LT voice → the leonas lane renders it (same breaker, own bucket).
        let lt = VoiceSpec {
            id: "lt-LT-LeonasNeural".into(),
            generator: "leonas".into(),
            lang: Lang::Lt,
        };
        assert!(matches!(
            d.speak(
                &item("labas"),
                &lt,
                1.0,
                2.0,
                2_000,
                &mut clock,
                &mut ledger,
                &mut sink
            ),
            SayOutcome::Spoke { .. }
        ));
        assert_eq!(
            (
                piper_renders.load(std::sync::atomic::Ordering::Relaxed),
                leonas_renders.load(std::sync::atomic::Ordering::Relaxed)
            ),
            (1, 1)
        );
        // Admitted generator with NO wired lane: loud drop, never silence.
        let ghost = VoiceSpec {
            id: "lt-LT-OnaNeural".into(),
            generator: "ona".into(),
            lang: Lang::Lt,
        };
        let out = d.speak(
            &item("nekalbu"),
            &ghost,
            1.0,
            3.0,
            3_000,
            &mut clock,
            &mut ledger,
            &mut sink,
        );
        match out {
            SayOutcome::Dropped { reason, anomaly } => {
                assert!(!anomaly);
                assert!(reason.contains("no ona lane wired"), "reason: {reason}");
            }
            other => panic!("expected drop, got {other:?}"),
        }
        assert_eq!(ledger.health().by_reason.get("render_error"), Some(&1));
    }
    #[test]
    fn idle_clock_bracket_covers_render_and_play() {
        let (lane, _renders) = FakeLane::piper(false);
        let mut sink = FakeSink {
            fail: false,
            plays: std::cell::Cell::new(0),
        };
        let mut clock = IdleClock::new();
        let mut ledger = DropLedger::new(None);
        let mut d = Dispatcher::new(vec![Box::new(lane)], "v1");
        let out = d.speak(
            &item("line"),
            &voice(),
            1.0,
            100.0,
            100_000,
            &mut clock,
            &mut ledger,
            &mut sink,
        );
        assert!(matches!(out, SayOutcome::Spoke { .. }));
        // Bracket closed (the real-elapsed width is ~0 in a test — the
        // invariant under test is the bracket STATE; the width arithmetic
        // is gramophone's own).
        assert!(!clock.speaking());
    }

    #[test]
    fn render_error_is_lossy_and_loud() {
        let (lane, _renders) = FakeLane::piper(true);
        let mut sink = FakeSink {
            fail: false,
            plays: std::cell::Cell::new(0),
        };
        let mut clock = IdleClock::new();
        let mut ledger = DropLedger::new(None);
        let mut d = Dispatcher::new(vec![Box::new(lane)], "v1");
        let out = d.speak(
            &item("lost words"),
            &voice(),
            1.0,
            5.0,
            5_000,
            &mut clock,
            &mut ledger,
            &mut sink,
        );
        assert_eq!(
            out,
            SayOutcome::Dropped {
                reason: "render: adapter: stub lane down".into(),
                anomaly: false
            }
        );
        let h: DropHealth = ledger.health();
        assert_eq!(h.undelivered, 1, "render_error is LOSSY");
        assert_eq!(h.by_reason.get("render_error"), Some(&1));
    }

    #[test]
    fn play_failure_is_process_error() {
        let (lane, _renders) = FakeLane::piper(false);
        let mut sink = FakeSink {
            fail: true,
            plays: std::cell::Cell::new(0),
        };
        let mut clock = IdleClock::new();
        let mut ledger = DropLedger::new(None);
        let mut d = Dispatcher::new(vec![Box::new(lane)], "v1");
        let out = d.speak(
            &item("unheard"),
            &voice(),
            1.0,
            5.0,
            5_000,
            &mut clock,
            &mut ledger,
            &mut sink,
        );
        assert_eq!(
            out,
            SayOutcome::Dropped {
                reason: "play: audio failed".into(),
                anomaly: false
            }
        );
        assert_eq!(ledger.health().by_reason.get("process_error"), Some(&1));
    }

    #[test]
    fn ga3_trip_is_anomaly_drop_and_cache_hit_costs_no_token() {
        // Capacity 1: the FIRST render drains the bucket; the second
        // (different text, cache miss) trips.
        let (lane, _renders) = FakeLane::piper(false);
        let mut sink = FakeSink {
            fail: false,
            plays: std::cell::Cell::new(0),
        };
        let mut clock = IdleClock::new();
        let mut ledger = DropLedger::new(None);
        let mut d = Dispatcher::new(vec![Box::new(lane)], "v1").with_breaker(BreakerConfig {
            capacity: 1,
            refill_per_min: 0,
            cooldown_ms: 60_000,
        });
        let v = voice();
        assert!(matches!(
            d.speak(
                &item("first"),
                &v,
                1.0,
                1.0,
                1_000,
                &mut clock,
                &mut ledger,
                &mut sink
            ),
            SayOutcome::Spoke { cache_hit: false }
        ));
        let out2 = d.speak(
            &item("second"),
            &v,
            1.0,
            2.0,
            2_000,
            &mut clock,
            &mut ledger,
            &mut sink,
        );
        match &out2 {
            SayOutcome::Dropped { reason, anomaly } => {
                assert!(*anomaly, "GA3 trip MUST surface the anomaly flag");
                assert!(
                    reason.starts_with("ga3 tripped"),
                    "reason says GA3: {reason}"
                );
            }
            other => panic!("expected drop, got {other:?}"),
        }
        assert_eq!(ledger.health().by_reason.get("render_error"), Some(&1));

        // Cache hit path consumes NO token: replay "first" while the
        // breaker is still tripped — the cache must carry it through.
        let out3 = d.speak(
            &item("first"),
            &v,
            1.0,
            3.0,
            3_000,
            &mut clock,
            &mut ledger,
            &mut sink,
        );
        assert_eq!(out3, SayOutcome::Spoke { cache_hit: true });
    }

    #[test]
    fn breaker_and_ledger_domains_documented_by_conversion() {
        assert_eq!(now_s_of(2_500), 2.5);
    }
}
