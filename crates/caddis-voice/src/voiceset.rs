//! voiceset.rs — per-label voice sets × auto-language routing (D3) with the
//! quorum R-B SPLIT-BY-PATH policy (T-35 verdict):
//!
//! - **Gated confirm phrases** — audio in the confirm path must be
//!   trustworthy: a missing voice HONESTLY DEGRADES (chime + panel +
//!   drop-ledger row). Never a substitute — a wrong voice reading a
//!   confirmation is an integrity risk (confirms are pre-rendered anyway).
//! - **General speech** — the nearest voice that CAN speak the detected
//!   language substitutes, with a DISTINCT warning earcon + ledger row:
//!   silence is the worst failure shape (drop-ledger doctrine).
//!
//! A label maps to a VOICE SET `{lt, en}`; detection picks WITHIN the set
//! (D3). The one banned outcome everywhere: an EN voice reading LT text —
//! the dead notice-swap's garbage never returns.

use crate::lang::Lang;
use crate::registry::Registry;

/// Which path an utterance is spoken on (R-B).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpeechPath {
    /// Pre-rendered gated confirm phrases — integrity path, honest degrade.
    GatedConfirm,
    /// General speech — substitution with warning is allowed.
    GeneralSpeech,
}

/// A label's voice set: the voice per language, `None` = label has none.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct VoiceSet {
    pub lt: Option<String>,
    pub en: Option<String>,
}

impl VoiceSet {
    pub fn new(lt: impl Into<String>, en: impl Into<String>) -> VoiceSet {
        VoiceSet {
            lt: Some(lt.into()),
            en: Some(en.into()),
        }
    }

    pub fn get(&self, lang: Lang) -> Option<&String> {
        match lang {
            Lang::Lt => self.lt.as_ref(),
            Lang::En => self.en.as_ref(),
        }
    }
}

/// What the gramophone should do with one resolved utterance.
#[derive(Debug, Clone, PartialEq)]
pub enum RouteDecision {
    /// Speak exactly this voice.
    Speak(String),
    /// Honest degrade: chime + panel line + drop-ledger row. No audio
    /// substitution is permitted on the path that asked for this.
    Degrade { reason: String },
    /// Substitute the nearest voice that speaks the language + fire the
    /// distinct SUBSTITUTED warning earcon + ledger row (R-B general path).
    Substitute { voice: String, warning: bool },
}

/// Resolve one spoken segment against a label's set (D3 + R-B).
///
/// `registry` supplies the substitute candidates (voices that speak the
/// detected language, registry order = deterministic preference). The
/// banned outcome (EN voice reading LT) is unreachable: a substitute always
/// speaks the detected language.
pub fn resolve(set: &VoiceSet, lang: Lang, path: SpeechPath, registry: &Registry) -> RouteDecision {
    if let Some(voice) = set.get(lang) {
        if registry.voice(voice).is_some() {
            return RouteDecision::Speak(voice.clone());
        }
        // Configured voice is not in the admitted registry — the config is
        // lying; treat as missing (fail toward the honest paths, never
        // toward an unverified voice id).
    }
    match path {
        SpeechPath::GatedConfirm => RouteDecision::Degrade {
            reason: format!(
                "gated-confirm: label has no admitted {:?} voice — honest degrade (R-B)",
                lang.as_str()
            ),
        },
        SpeechPath::GeneralSpeech => {
            let candidates = registry.voices_for(lang);
            match candidates.first() {
                Some(v) => RouteDecision::Substitute {
                    voice: v.clone(),
                    warning: true,
                },
                None => RouteDecision::Degrade {
                    reason: format!(
                        "general: no admitted voice speaks {:?} at all — honest degrade (R-B)",
                        lang.as_str()
                    ),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::{GeneratorSpec, Lane, VoiceSpec};

    fn test_registry() -> Registry {
        let mut r = Registry::default();
        r.admit(GeneratorSpec {
            id: "piper".into(),
            lane: Lane::Offline,
            startup_cap_ms: 50,
            render_cap_ms: 1500,
            declared_endpoints: vec![],
        })
        .unwrap();
        r.admit(GeneratorSpec {
            id: "leonas".into(),
            lane: Lane::Network,
            startup_cap_ms: 100,
            render_cap_ms: 1500,
            declared_endpoints: vec!["wss://speech.platform.bing.com".into()],
        })
        .unwrap();
        r.attach(VoiceSpec {
            id: "en_US-ryan".into(),
            generator: "piper".into(),
            lang: Lang::En,
        })
        .unwrap();
        r.attach(VoiceSpec {
            id: "en_US-amy".into(),
            generator: "piper".into(),
            lang: Lang::En,
        })
        .unwrap();
        r.attach(VoiceSpec {
            id: "lt-LT-LeonasNeural".into(),
            generator: "leonas".into(),
            lang: Lang::Lt,
        })
        .unwrap();
        r
    }

    #[test]
    fn present_voice_speaks() {
        let r = test_registry();
        let set = VoiceSet::new("lt-LT-LeonasNeural", "en_US-ryan");
        assert_eq!(
            resolve(&set, Lang::Lt, SpeechPath::GatedConfirm, &r),
            RouteDecision::Speak("lt-LT-LeonasNeural".into())
        );
        assert_eq!(
            resolve(&set, Lang::En, SpeechPath::GeneralSpeech, &r),
            RouteDecision::Speak("en_US-ryan".into())
        );
    }

    #[test]
    fn confirm_path_never_substitutes() {
        let r = test_registry();
        let en_only = VoiceSet {
            lt: None,
            en: Some("en_US-ryan".into()),
        };
        let d = resolve(&en_only, Lang::Lt, SpeechPath::GatedConfirm, &r);
        assert!(matches!(d, RouteDecision::Degrade { .. }), "{d:?}");
    }

    #[test]
    fn general_path_substitutes_with_warning() {
        let r = test_registry();
        let en_only = VoiceSet {
            lt: None,
            en: Some("en_US-ryan".into()),
        };
        let d = resolve(&en_only, Lang::Lt, SpeechPath::GeneralSpeech, &r);
        assert_eq!(
            d,
            RouteDecision::Substitute {
                voice: "lt-LT-LeonasNeural".into(),
                warning: true
            }
        );
    }

    #[test]
    fn no_voice_at_all_degrades_on_both_paths() {
        let r = test_registry();
        // Strip LT voices to empty the substitute pool.
        let mut r2 = r.clone();
        r2.voices.retain(|v| v.lang != Lang::Lt);
        let set = VoiceSet {
            lt: None,
            en: Some("en_US-ryan".into()),
        };
        for path in [SpeechPath::GatedConfirm, SpeechPath::GeneralSpeech] {
            assert!(
                matches!(
                    resolve(&set, Lang::Lt, path, &r2),
                    RouteDecision::Degrade { .. }
                ),
                "{path:?}"
            );
        }
    }

    #[test]
    fn substitute_always_speaks_the_detected_language() {
        // The banned outcome (EN voice reading LT) is unreachable by shape:
        // candidates come from voices_for(lang).
        let r = test_registry();
        for lang in [Lang::Lt, Lang::En] {
            let empty = VoiceSet::default();
            if let RouteDecision::Substitute { voice, .. } =
                resolve(&empty, lang, SpeechPath::GeneralSpeech, &r)
            {
                assert_eq!(r.voice(&voice).unwrap().lang, lang);
            }
        }
    }

    #[test]
    fn phantom_config_voice_fails_honest_not_phantom_speak() {
        let r = test_registry();
        let lying = VoiceSet {
            lt: Some("lt-LT-Ghost".into()),
            en: None,
        };
        assert!(matches!(
            resolve(&lying, Lang::Lt, SpeechPath::GeneralSpeech, &r),
            RouteDecision::Substitute { .. }
        ));
        assert!(matches!(
            resolve(&lying, Lang::Lt, SpeechPath::GatedConfirm, &r),
            RouteDecision::Degrade { .. }
        ));
    }
}
