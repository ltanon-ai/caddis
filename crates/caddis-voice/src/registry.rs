//! registry.rs — the generator/voice registry as CONFIG DATA (D2, F-A4),
//! with v1 admission (R-E): ONLY organ-native compiled generators load —
//! `piper` (offline EN), `leonas` + `ona` (network LT, direct edge-tts).
//! There is NO external-adapter loading path in v1: an unknown id is
//! rejected by construction (fail-closed — nothing unsigned can load
//! because nothing external CAN load). The day external adapters open,
//! admission re-enters deliberation (T-35 verdict R-E).
//!
//! F-A4 admission proofs: latency caps declared within bounds
//! (startup ≤ 100ms, render ≤ 1500ms), and GA1 per-adapter
//! DECLARED-ENDPOINT allowlist — network generators declare their exact
//! egress endpoints; the organ dials ONLY declared endpoints. Offline
//! generators must declare NONE.

use crate::json::Value;
use crate::lang::Lang;

/// The v1 internal admission set (R-E). New citizens require a deliberate
/// code change + ladder, never a config edit.
pub const INTERNAL_GENERATORS: [&str; 3] = ["piper", "leonas", "ona"];

/// F-A4 latency-cap ceilings a generator must declare within.
pub const MAX_STARTUP_CAP_MS: u32 = 100;
pub const MAX_RENDER_CAP_MS: u32 = 1500;

/// Which lane a generator renders on (D8: lanes soak separately).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lane {
    /// Local offline render (piper). No egress, ever.
    Offline,
    /// Network render (direct edge-tts). Egress ONLY to declared endpoints.
    Network,
}

/// One generator (voice source) in the arsenal.
#[derive(Debug, Clone, PartialEq)]
pub struct GeneratorSpec {
    pub id: String,
    pub lane: Lane,
    /// F-A4: declared startup latency cap (ms).
    pub startup_cap_ms: u32,
    /// F-A4: declared render latency cap (ms).
    pub render_cap_ms: u32,
    /// GA1: the exact endpoints this generator may dial. Empty for Offline.
    pub declared_endpoints: Vec<String>,
}

/// One voice attached to a generator.
#[derive(Debug, Clone, PartialEq)]
pub struct VoiceSpec {
    pub id: String,
    pub generator: String,
    pub lang: Lang,
}

/// The parsed, admitted registry.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Registry {
    pub generators: Vec<GeneratorSpec>,
    pub voices: Vec<VoiceSpec>,
}

/// Admission/parse failures. Fail-closed: the registry is either fully
/// admitted or rejected with a reason.
#[derive(Debug, Clone, PartialEq)]
pub struct RegistryErr(pub String);

impl std::fmt::Display for RegistryErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "registry: {}", self.0)
    }
}

fn err<T>(msg: impl Into<String>) -> Result<T, RegistryErr> {
    Err(RegistryErr(msg.into()))
}

impl Registry {
    /// Admit one generator against the v1 rules (R-E + F-A4 + GA1).
    pub fn admit(&mut self, spec: GeneratorSpec) -> Result<(), RegistryErr> {
        if !INTERNAL_GENERATORS.contains(&spec.id.as_str()) {
            return err(format!(
                "generator '{}' is not in the internal admission set {:?} — v1 admits ONLY organ-native generators (R-E); external adapters do not load",
                spec.id, INTERNAL_GENERATORS
            ));
        }
        if self.generators.iter().any(|g| g.id == spec.id) {
            return err(format!("generator '{}' admitted twice", spec.id));
        }
        if spec.startup_cap_ms == 0 || spec.startup_cap_ms > MAX_STARTUP_CAP_MS {
            return err(format!(
                "generator '{}': startup_cap_ms {} outside F-A4 bounds 1..={}",
                spec.id, spec.startup_cap_ms, MAX_STARTUP_CAP_MS
            ));
        }
        if spec.render_cap_ms == 0 || spec.render_cap_ms > MAX_RENDER_CAP_MS {
            return err(format!(
                "generator '{}': render_cap_ms {} outside F-A4 bounds 1..={}",
                spec.id, spec.render_cap_ms, MAX_RENDER_CAP_MS
            ));
        }
        match spec.lane {
            Lane::Offline => {
                if !spec.declared_endpoints.is_empty() {
                    return err(format!(
                        "offline generator '{}' declares endpoints {:?} — an offline lane has no egress (GA1)",
                        spec.id, spec.declared_endpoints
                    ));
                }
            }
            Lane::Network => {
                if spec.declared_endpoints.is_empty() {
                    return err(format!(
                        "network generator '{}' declares NO endpoints — GA1 requires the exact egress allowlist",
                        spec.id
                    ));
                }
                for ep in &spec.declared_endpoints {
                    validate_endpoint(&spec.id, ep)?;
                }
            }
        }
        self.generators.push(spec);
        Ok(())
    }

    /// Attach a voice; its generator must already be admitted.
    pub fn attach(&mut self, voice: VoiceSpec) -> Result<(), RegistryErr> {
        if !self.generators.iter().any(|g| g.id == voice.generator) {
            return err(format!(
                "voice '{}' references unknown generator '{}'",
                voice.id, voice.generator
            ));
        }
        if self.voices.iter().any(|v| v.id == voice.id) {
            return err(format!("voice '{}' attached twice", voice.id));
        }
        self.voices.push(voice);
        Ok(())
    }

    pub fn generator(&self, id: &str) -> Option<&GeneratorSpec> {
        self.generators.iter().find(|g| g.id == id)
    }

    pub fn voice(&self, id: &str) -> Option<&VoiceSpec> {
        self.voices.iter().find(|v| v.id == id)
    }

    /// All voice ids able to speak `lang`, in registry order (the
    /// substitute-with-warning candidates for R-B resolution).
    pub fn voices_for(&self, lang: Lang) -> Vec<String> {
        self.voices
            .iter()
            .filter(|v| v.lang == lang)
            .map(|v| v.id.clone())
            .collect()
    }

    /// Parse + fully admit a registry from its JSON value.
    pub fn from_value(v: &Value) -> Result<Registry, RegistryErr> {
        let mut reg = Registry::default();
        let gens = v
            .get("generators")
            .and_then(Value::as_arr)
            .ok_or_else(|| RegistryErr("missing 'generators' array".into()))?;
        for g in gens {
            reg.admit(parse_generator(g)?)?;
        }
        if let Some(voices) = v.get("voices").and_then(Value::as_arr) {
            for voice in voices {
                reg.attach(parse_voice(voice)?)?;
            }
        }
        if reg.generators.is_empty() {
            return err("registry admits no generators");
        }
        Ok(reg)
    }
}

fn parse_generator(v: &Value) -> Result<GeneratorSpec, RegistryErr> {
    let id = v
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| RegistryErr("generator entry missing 'id'".into()))?
        .to_string();
    let lane = match v.get("lane").and_then(Value::as_str) {
        Some("offline") => Lane::Offline,
        Some("network") => Lane::Network,
        other => {
            return err(format!(
                "generator '{id}': bad lane {other:?} (offline|network)"
            ))
        }
    };
    let startup_cap_ms = num(v, "startup_cap_ms", &id)?;
    let render_cap_ms = num(v, "render_cap_ms", &id)?;
    let endpoints = v
        .get("declared_endpoints")
        .and_then(Value::as_arr)
        .map(|a| {
            a.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    Ok(GeneratorSpec {
        id,
        lane,
        startup_cap_ms,
        render_cap_ms,
        declared_endpoints: endpoints,
    })
}

fn num(v: &Value, k: &str, id: &str) -> Result<u32, RegistryErr> {
    v.get(k)
        .and_then(Value::as_f64)
        .map(|n| n as u32)
        .ok_or_else(|| RegistryErr(format!("generator '{id}': missing '{k}'")))
}

fn parse_voice(v: &Value) -> Result<VoiceSpec, RegistryErr> {
    let id = v
        .get("id")
        .and_then(Value::as_str)
        .ok_or_else(|| RegistryErr("voice entry missing 'id'".into()))?
        .to_string();
    let generator = v
        .get("generator")
        .and_then(Value::as_str)
        .ok_or_else(|| RegistryErr(format!("voice '{id}': missing 'generator'")))?
        .to_string();
    let lang = v
        .get("lang")
        .and_then(Value::as_str)
        .ok_or_else(|| RegistryErr(format!("voice '{id}': missing 'lang'")))?
        .parse::<Lang>()
        .map_err(|_| RegistryErr(format!("voice '{id}': bad 'lang'")))?;
    Ok(VoiceSpec {
        id,
        generator,
        lang,
    })
}

/// GA1 endpoint shape: https/wss, host present, no embedded credentials,
/// no whitespace. (The organ dialing ONLY these is enforced by the adapter
/// layer in P2; day-one validation keeps the allowlist honest.)
fn validate_endpoint(gen: &str, ep: &str) -> Result<(), RegistryErr> {
    let scheme_ok = ep.starts_with("https://") || ep.starts_with("wss://");
    if !scheme_ok {
        return err(format!(
            "generator '{gen}': endpoint '{ep}' must be https:// or wss:// (GA1)"
        ));
    }
    if ep.contains('@') {
        return err(format!(
            "generator '{gen}': endpoint '{ep}' embeds credentials (GA1: host only)"
        ));
    }
    if ep.chars().any(char::is_whitespace) {
        return err(format!(
            "generator '{gen}': endpoint '{ep}' contains whitespace (GA1)"
        ));
    }
    let host = ep
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or("")
        .split(['/', '?'])
        .next()
        .unwrap_or("");
    if host.is_empty() || host.contains(char::is_whitespace) {
        return err(format!(
            "generator '{gen}': endpoint '{ep}' has no clean host"
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn piper() -> GeneratorSpec {
        GeneratorSpec {
            id: "piper".into(),
            lane: Lane::Offline,
            startup_cap_ms: 50,
            render_cap_ms: 1500,
            declared_endpoints: vec![],
        }
    }

    fn leonas() -> GeneratorSpec {
        GeneratorSpec {
            id: "leonas".into(),
            lane: Lane::Network,
            startup_cap_ms: 100,
            render_cap_ms: 1500,
            declared_endpoints: vec!["wss://speech.platform.bing.com".into()],
        }
    }

    #[test]
    fn internal_generators_admit() {
        let mut r = Registry::default();
        r.admit(piper()).unwrap();
        r.admit(leonas()).unwrap();
        r.admit(GeneratorSpec {
            id: "ona".into(),
            lane: Lane::Network,
            startup_cap_ms: 100,
            render_cap_ms: 1500,
            declared_endpoints: vec!["wss://speech.platform.bing.com".into()],
        })
        .unwrap();
        assert_eq!(r.generators.len(), 3);
    }

    #[test]
    fn unknown_generator_is_rejected_fail_closed() {
        let mut r = Registry::default();
        let e = r
            .admit(GeneratorSpec {
                id: "elevenlabs".into(),
                lane: Lane::Network,
                startup_cap_ms: 50,
                render_cap_ms: 500,
                declared_endpoints: vec!["https://api.example.com".into()],
            })
            .unwrap_err();
        assert!(e.0.contains("R-E"), "{e}");
    }

    #[test]
    fn caps_outside_fa4_bounds_reject() {
        let mut r = Registry::default();
        let mut g = piper();
        g.startup_cap_ms = 101;
        assert!(r.admit(g).unwrap_err().0.contains("F-A4"));
        let mut g = piper();
        g.render_cap_ms = 1501;
        assert!(r.admit(g).unwrap_err().0.contains("F-A4"));
        let mut g = piper();
        g.render_cap_ms = 0;
        assert!(r.admit(g).is_err());
    }

    #[test]
    fn offline_generator_must_not_declare_egress() {
        let mut r = Registry::default();
        let mut g = piper();
        g.declared_endpoints = vec!["https://evil.example".into()];
        assert!(r.admit(g).unwrap_err().0.contains("GA1"));
    }

    #[test]
    fn network_generator_must_declare_endpoints() {
        let mut r = Registry::default();
        let mut g = leonas();
        g.declared_endpoints = vec![];
        assert!(r.admit(g).unwrap_err().0.contains("GA1"));
    }

    #[test]
    fn bad_endpoint_shapes_reject() {
        let mut r = Registry::default();
        for bad in [
            "http://plain.example",
            "wss://h.example/u?p=1 with space",
            "https://user:pw@h.example",
        ] {
            let mut g = leonas();
            g.declared_endpoints = vec![bad.into()];
            assert!(r.admit(g.clone()).is_err(), "accepted {bad}");
        }
        // Clean ones pass.
        let mut g = leonas();
        g.declared_endpoints = vec!["https://speech.example.com/path".into()];
        assert!(r.admit(g).is_ok());
    }

    #[test]
    fn duplicate_ids_reject() {
        let mut r = Registry::default();
        r.admit(piper()).unwrap();
        assert!(r.admit(piper()).is_err());
    }

    #[test]
    fn voices_attach_and_lookup() {
        let mut r = Registry::default();
        r.admit(leonas()).unwrap();
        r.admit(piper()).unwrap();
        r.attach(VoiceSpec {
            id: "lt-LT-LeonasNeural".into(),
            generator: "leonas".into(),
            lang: Lang::Lt,
        })
        .unwrap();
        assert!(r
            .attach(VoiceSpec {
                id: "lt-LT-OnaNeural".into(),
                generator: "ona".into(),
                lang: Lang::Lt
            })
            .is_err()); // 'ona' generator not admitted yet — attach must fail closed
        r.attach(VoiceSpec {
            id: "en_US-ryan".into(),
            generator: "piper".into(),
            lang: Lang::En,
        })
        .unwrap();
        assert_eq!(r.voices_for(Lang::Lt), vec!["lt-LT-LeonasNeural"]);
        assert_eq!(r.voices_for(Lang::En), vec!["en_US-ryan"]);
        // Voice referencing unknown generator is rejected.
        assert!(r
            .attach(VoiceSpec {
                id: "xx".into(),
                generator: "nope".into(),
                lang: Lang::En
            })
            .is_err());
    }

    #[test]
    fn parses_from_json_value() {
        let text = r#"{
            "generators": [
                {"id": "piper", "lane": "offline", "startup_cap_ms": 50, "render_cap_ms": 1500, "declared_endpoints": []},
                {"id": "leonas", "lane": "network", "startup_cap_ms": 100, "render_cap_ms": 1500,
                 "declared_endpoints": ["wss://speech.platform.bing.com"]}
            ],
            "voices": [
                {"id": "en_US-ryan", "generator": "piper", "lang": "en"},
                {"id": "en_US-amy", "generator": "piper", "lang": "en"},
                {"id": "lt-LT-LeonasNeural", "generator": "leonas", "lang": "lt"}
            ]
        }"#;
        let v = crate::json::parse(text).unwrap();
        let r = Registry::from_value(&v).unwrap();
        assert_eq!(r.generators.len(), 2);
        assert_eq!(r.voices.len(), 3);
        assert_eq!(r.voices_for(Lang::Lt), vec!["lt-LT-LeonasNeural"]);
    }

    #[test]
    fn parse_failures_carry_context() {
        let v = crate::json::parse(r#"{"generators": []}"#).unwrap();
        assert!(Registry::from_value(&v)
            .unwrap_err()
            .0
            .contains("no generators"));
        let v = crate::json::parse(r#"{"voices": []}"#).unwrap();
        assert!(Registry::from_value(&v)
            .unwrap_err()
            .0
            .contains("generators"));
    }
}
