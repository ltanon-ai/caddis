//! caddis-voice — the CADDIS VOICE ORGAN (gramophone speaks, horn hears).
//!
//! **P1 slice (a) — organ core** (sergeant tick 2026-08-27, T-35 verdict
//! rung 6: `state/briefs/t35-voice-amendment-quorum/VERDICT.md`). The
//! amendment ladder (brief → council F-A1..F-A8 → quorum GA1-GA3/R-B/R-D/
//! R-E/R-F → rung 6) is COMPLETE; this crate is its first build increment:
//!
//! - [`detect`] — F-A1 language detection ladder: L0 label default →
//!   L1 diacritic scan → L2 offline trigram, ACTIVE with a hard 30ms cap,
//!   per-decision telemetry, text-hash cache, ambiguity tie-break → LT.
//!   No LLM anywhere in detection.
//! - [`voiceset`] — D3 per-label voice sets `{lt,en}` with the R-B
//!   split-by-path policy: gated confirms honestly degrade; general speech
//!   substitutes with a warning. An EN voice reading LT text is
//!   unreachable by shape — the notice-swap garbage stays dead.
//! - [`registry`] — the arsenal as config data (D2): v1 admits ONLY the
//!   organ-native generators `piper` / `leonas` / `ona` (R-E fail-closed —
//!   no external-adapter loading path exists), F-A4 latency caps, GA1
//!   declared-endpoint allowlist for network lanes.
//! - [`config`] — D6 day-one shape: file-based config, defaults per
//!   language, per-label sets, generator enable flags, the carried 2.5s
//!   LT network deadline (R-D).
//!
//! **P1 slice (b)** (same tick, second increment — the OS-facing half):
//! - [`mutex`] — the PORT HARD MUTEX: exact-port exclusive bind, port 0
//!   categorically refused (the ephemeral fallback that once hid a
//!   dual-daemon defect can never recur), kernel-held, Drop-released.
//! - [`health`] — GET /health over the mutex-held listener: organ identity,
//!   uptime, ports held, `spawned_children` counter, and the VRAM capacity
//!   report (QQ2: the report exists BEFORE any spawn can — a fresh organ
//!   serves it with `spawned_children: 0`).
//! - [`vram`] + [`platform`] — DXGI adapter enumeration via the raw
//!   `CreateDXGIFactory1` export (std-only FFI law, winprobe precedent);
//!   failopen-REPORT: unknown is stated, never invented.
//! - [`job`] — the Job Objects harness: [`job::DeadManSwitch`] arms
//!   kill-on-close for the organ process (children die with it, handle
//!   intentionally leaked — the leak IS the switch); [`job::ChildScope`]
//!   closes on Drop (the kill-now supervision primitive, kernel-proven in
//!   tests).
//! - [`configio`] — config file loading (missing = embedded boot, corrupt =
//!   LOUD) and the WARDEN-GATED write path: the estate's stdin-frame seam
//!   (`tool/command/path/content`, byte-exact lengths), fail-closed verdict
//!   parse, `allow`+`seq>0` required, atomic tmp+rename landing.
//!
//! **P2 — the LISTENING HORN** (this slice): [`horn`] adopt-don't-duplicate
//! supervision of whisper-server.exe (port-bound liveness, pid+image
//! identity, strangers refused loudly, adopted engines never killed),
//! [`guards`] token+Host+size gates, [`multipart`] the tiny dialect both
//! HTTP directions speak, [`whisperc`] the engine lane client with GA2
//! response validation, [`transcribe`] the daemon-contract endpoint
//! (single-flight 429, WAV sanity, allowlisted `path`), and [`httpd`] the
//! capped thread-per-connection surface (/health + /transcribe).
//!
//! **P2 — the ADAPTER half** (next slice): [`adapter`] the guard layer
//! every render passes (GA1 dial-time endpoint authorization, GA2 audio
//! validation, GA3 circuit breaker, markup/secret text sanitization),
//! [`piper`] the offline EN adapter driving the daemon-proven CLI under a
//! kill-on-close job, [`edgetts`] the direct edge-tts protocol (DRM
//! token, GA1-gated dial URL, SSML assembly, frame parsing, R-D
//! deadline) over the [`edgetts::WsStream`] seam — the schannel TLS/WSS
//! fail-closed (LIVE-PROVEN over :443, slice d). [`earcons`] the earcon
//! set as data: four motifs ported verbatim from the daemon + the R-B
//! verdict's distinct `substituted` warning and `degrade` chime, with a
//! mechanical distinctness law.
//!
//! **P3 — the GRAMOPHONE core** (this slice): [`gramophone`] the say
//! queue (2 s same-key coalesce with critical exemption, hard cap 24 with
//! oldest-non-critical eviction, per-class due delays, CUE-ONLY staleness
//! measured on the idle clock), the [`gramophone::IdleClock`] (speech
//! time credited back; the 180 s utterance wedge backstop), the
//! [`gramophone::DropLedger`] (a drop must be LOUD: per-reason counts,
//! lossy vs by-design, undelivered text persisted to JSONL), and the
//! [`gramophone::WavCache`] (the daemon's proven composite key over a
//! byte-budget LRU). All four are ports of the daemon's proven
//! operator-reliability organs (scheduler.py / idle_clock.py /
//! drop_ledger.py / tts.py) — pure arithmetic on a caller-supplied clock.
//!
//! **P3 slice (b)** — playback + dispatch (this slice): [`play`] the
//! KILLABLE PLAY CHILD (audio.py + play_proc.py ported onto winmm
//! `waveOut`: one short-lived child per attempt, per device view, exit
//! contract 0/10/20/30/40, default-device sentinels re-resolved in the
//! fresh child, deadline = duration + the daemon's measured 15 s
//! budget, wedged child KILLED) and [`say`] the dispatch bracket
//! (scheduler_emit.py `_speak`): cache-first, GA3 breaker gating the
//! lane (a trip is an ANOMALY drop, ledger-recorded), render → GA2 →
//! play — all inside the idle-clock SPEAKING bracket so queued speech
//! never ages.
//!
//! **P3 slice (c)** — the service half: [`earcons`] grows WAV synthesis
//! (additive, phase-accumulated chirp, attack/decay envelope, stereo
//! shape, peak-normalized to `peak_dbfs`) so the six motifs become real
//! audio; [`sayd`] assembles the gramophone end-to-end — queue →
//! dispatch → play child — with the QQ5 earcon wiring (attention/done/
//! fail life events + the R-B `substituted`/`degrade` chimes), the R-B
//! routing decisions firing them, and the `SayService` thread shell the
//! httpd talks to; [`httpd`] grows the `/say` and `/earcon` routes.
//! Cutover + soak are P4-P5.

pub mod adapter;
pub mod config;
pub mod configio;
pub mod detect;
pub mod earcons;
pub mod edgetts;
pub mod gramophone;
pub mod guards;
pub mod health;
pub mod horn;
pub mod httpd;
pub mod job;
pub mod json;
pub mod lang;
pub mod multipart;
pub mod mutex;
pub mod piper;
pub mod play;
pub mod platform;
pub mod registry;
pub mod say;
pub mod sayd;
pub mod sha1;
pub mod sha256;
pub mod transcribe;
pub mod trigram;
pub mod voiceset;
pub mod vram;
pub mod whisperc;
pub mod wss;

pub use adapter::{
    authorize_dial, sanitize_text, validate_mp3, validate_wav, AdapterErr, AudioFormat, Breaker,
    BreakerConfig, RenderedAudio,
};
pub use config::{OrganConfig, DEFAULT_CONFIG_JSON};
pub use configio::{
    load_config, save_config_document, ConfigSource, RealWarden, SaveOutcome, Warden,
    WardenVerdict, COMMAND as WARDEN_COMMAND, TOOL as WARDEN_TOOL,
};
pub use detect::{CacheStats, Decision, DetectOptions, Detector, Layer, Segment, Utterance};
pub use earcons::{parse_set as parse_earcon_set, EarconSet, Motif, EARCON_SET_JSON};
pub use edgetts::{dial_url, sec_ms_gec, synthesize as edge_synthesize, Prosody, SessionOpts};
pub use gramophone::{
    wav_cache_key, Admission, CachedAudio, DropHealth, DropLedger, IdleClock, SayItem, SayQueue,
    WavCache, LOSSY_REASONS,
};
pub use guards::{GuardVerdict, TokenGuard, MAX_UPLOAD_BYTES};
pub use health::{route as route_health, HealthState, Response as HealthResponse};
pub use horn::{
    adopt_decision, backoff_for, EngineWorld, HornSettings, HornState, OsEngineWorld, Supervisor,
    TickReport,
};
pub use httpd::{route as route_organ, OrganRoutes};
pub use job::{ChildScope, DeadManSwitch, JobErr};
pub use lang::Lang;
pub use mutex::{bind_exclusive, PortMutexErr};
pub use piper::{PiperAdapter, PiperPaths, PIPER_KILL_DEADLINE_MS};
pub use play::{
    is_default as is_default_device, read_wav, fit_channels, resample, AudioOut, Pcm,
    PlaybackOutcome, PlayFail, EXIT_BAD_INPUT, EXIT_NO_VIEW, EXIT_OK, EXIT_OPEN_FAILED,
    EXIT_PLAY_FAILED, PLAY_STARTUP_BUDGET_S, TIMEOUT_RC,
};
pub use registry::{GeneratorSpec, Lane, Registry, RegistryErr, VoiceSpec, INTERNAL_GENERATORS};
pub use say::{Dispatcher, PlaySink, RenderLane, SayOutcome};
pub use sayd::{EarconPlayer, SayCounts, SayService, Worker as SayWorker};
pub use earcons::{synth_wav as synth_earcon_wav, EARCON_SAMPLE_RATE};
pub use transcribe::{HornService, WavMeta, ENGINE_NAME, MIN_AUDIO_S};
pub use voiceset::{RouteDecision, SpeechPath, VoiceSet};
pub use vram::{probe as probe_vram, AdapterMem, VramReport};
pub use whisperc::{transcribe as engine_transcribe, Transcript, WhisperErr};

pub const VERSION: &str = "0.1.0";

#[cfg(test)]
mod tests {
    use super::*;

    /// The full P1 core flow as one story: config → detection → routing,
    /// both R-B paths, exactly as the verdict describes them.
    #[test]
    fn core_flow_config_detect_route() {
        let cfg = OrganConfig::default();
        let mut det = Detector::new(DetectOptions::default());
        let sgt = &cfg.labels["sergeant"];

        // Marked LT utterance → L1 → Leonas speaks (confirm path OK).
        let u = det.detect("Patvirtinta, užduotis įvykdyta.", sgt.declared);
        assert_eq!(u.segments[0].decision.layer, Layer::L1Diacritic);
        let d = voiceset::resolve(
            &sgt.set,
            u.segments[0].decision.lang,
            SpeechPath::GatedConfirm,
            &cfg.registry,
        );
        assert_eq!(d, RouteDecision::Speak("lt-LT-LeonasNeural".into()));

        // Plain EN utterance → L2 → ryan.
        let u = det.detect("The pipeline is green, deploying now.", sgt.declared);
        assert_eq!(u.segments[0].decision.lang, Lang::En);
        let d = voiceset::resolve(
            &sgt.set,
            u.segments[0].decision.lang,
            SpeechPath::GeneralSpeech,
            &cfg.registry,
        );
        assert_eq!(d, RouteDecision::Speak("en_US-ryan".into()));

        // A label with no LT voice: confirm degrades honestly, general
        // substitutes Leonas WITH warning.
        let en_only = VoiceSet {
            lt: None,
            en: Some("en_US-ryan".into()),
        };
        assert!(matches!(
            voiceset::resolve(&en_only, Lang::Lt, SpeechPath::GatedConfirm, &cfg.registry),
            RouteDecision::Degrade { .. }
        ));
        assert_eq!(
            voiceset::resolve(&en_only, Lang::Lt, SpeechPath::GeneralSpeech, &cfg.registry),
            RouteDecision::Substitute {
                voice: "lt-LT-LeonasNeural".into(),
                warning: true
            }
        );

        // Mixed utterance is detected as mixed — per-segment voices.
        let u = det.detect("Sveiki, draugai. Hello everyone. Ačiū už dėmesį.", None);
        assert!(u.mixed);
    }
}
