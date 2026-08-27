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
//! Engines (piper/edge-tts) are P2; the gramophone queue/earcons are P3;
//! cutover + soak are P4-P5.
//!
//! Zero runtime dependencies, sync, std only (organ material law).

pub mod config;
pub mod configio;
pub mod detect;
pub mod health;
pub mod job;
pub mod json;
pub mod lang;
pub mod mutex;
pub mod platform;
pub mod registry;
pub mod trigram;
pub mod voiceset;
pub mod vram;

pub use config::{OrganConfig, DEFAULT_CONFIG_JSON};
pub use configio::{
    load_config, save_config_document, ConfigSource, RealWarden, SaveOutcome, Warden, WardenVerdict,
    COMMAND as WARDEN_COMMAND, TOOL as WARDEN_TOOL,
};
pub use detect::{CacheStats, Decision, DetectOptions, Detector, Layer, Segment, Utterance};
pub use health::{route as route_health, HealthState, Response as HealthResponse};
pub use job::{ChildScope, DeadManSwitch, JobErr};
pub use lang::Lang;
pub use mutex::{bind_exclusive, PortMutexErr};
pub use registry::{GeneratorSpec, Lane, Registry, RegistryErr, VoiceSpec, INTERNAL_GENERATORS};
pub use voiceset::{RouteDecision, SpeechPath, VoiceSet};
pub use vram::{probe as probe_vram, AdapterMem, VramReport};

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
        let d = voiceset::resolve(&sgt.set, u.segments[0].decision.lang, SpeechPath::GatedConfirm, &cfg.registry);
        assert_eq!(d, RouteDecision::Speak("lt-LT-LeonasNeural".into()));

        // Plain EN utterance → L2 → ryan.
        let u = det.detect("The pipeline is green, deploying now.", sgt.declared);
        assert_eq!(u.segments[0].decision.lang, Lang::En);
        let d = voiceset::resolve(&sgt.set, u.segments[0].decision.lang, SpeechPath::GeneralSpeech, &cfg.registry);
        assert_eq!(d, RouteDecision::Speak("en_US-ryan".into()));

        // A label with no LT voice: confirm degrades honestly, general
        // substitutes Leonas WITH warning.
        let en_only = VoiceSet { lt: None, en: Some("en_US-ryan".into()) };
        assert!(matches!(
            voiceset::resolve(&en_only, Lang::Lt, SpeechPath::GatedConfirm, &cfg.registry),
            RouteDecision::Degrade { .. }
        ));
        assert_eq!(
            voiceset::resolve(&en_only, Lang::Lt, SpeechPath::GeneralSpeech, &cfg.registry),
            RouteDecision::Substitute { voice: "lt-LT-LeonasNeural".into(), warning: true }
        );

        // Mixed utterance is detected as mixed — per-segment voices.
        let u = det.detect("Sveiki, draugai. Hello everyone. Ačiū už dėmesį.", None);
        assert!(u.mixed);
    }
}
