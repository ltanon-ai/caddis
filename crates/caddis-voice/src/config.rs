//! config.rs — the organ's file-based configuration (D6, day-one shape).
//!
//! File-based config; the world Voice Booth gets a READ view; ALL writes
//! are warden-gated (slice (b) — this slice defines + validates the shape).
//! The config carries: the registry (as data), default voices per language,
//! per-label voice sets, generator enable flags, and the carried deadlines
//! (R-D: 2.5s single-attempt LT network deadline; offline lane has no
//! network deadline by design).

use crate::json::{self, Value};
use crate::lang::Lang;
use crate::registry::{Registry, RegistryErr};
use crate::voiceset::VoiceSet;
use std::collections::BTreeMap;

/// R-D: the quorum-carried single-attempt network deadline for LT speech.
pub const DEFAULT_LT_NETWORK_DEADLINE_MS: u32 = 2500;

/// One label's routing entry.
#[derive(Debug, Clone, PartialEq)]
pub struct LabelConfig {
    /// Label-declared default language (L0). `None` = undeclared.
    pub declared: Option<Lang>,
    pub set: VoiceSet,
}

/// The whole organ config, parsed + validated.
#[derive(Debug, Clone, PartialEq)]
pub struct OrganConfig {
    pub registry: Registry,
    pub defaults: BTreeMap<Lang, String>,
    pub labels: BTreeMap<String, LabelConfig>,
    pub generators_enabled: BTreeMap<String, bool>,
    pub lt_network_deadline_ms: u32,
}

#[derive(Debug, Clone, PartialEq)]
pub struct ConfigErr(pub String);

impl std::fmt::Display for ConfigErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "config: {}", self.0)
    }
}

/// The day-one default arsenal (operator amendment pt 1-3): piper offline
/// (ryan + amy, EN), leonas network (LT male), ona network (LT female) —
/// the widest admitted v1 set. Served as an embedded JSON document so the
/// organ boots with a valid config even before any file exists.
pub const DEFAULT_CONFIG_JSON: &str = r#"{
    "generators": [
        {"id": "piper",   "lane": "offline", "startup_cap_ms": 50,  "render_cap_ms": 1500, "declared_endpoints": []},
        {"id": "leonas",  "lane": "network", "startup_cap_ms": 100, "render_cap_ms": 1500,
         "declared_endpoints": ["wss://speech.platform.bing.com"]},
        {"id": "ona",     "lane": "network", "startup_cap_ms": 100, "render_cap_ms": 1500,
         "declared_endpoints": ["wss://speech.platform.bing.com"]}
    ],
    "voices": [
        {"id": "en_US-ryan",           "generator": "piper",   "lang": "en"},
        {"id": "en_US-amy",            "generator": "piper",   "lang": "en"},
        {"id": "lt-LT-LeonasNeural",   "generator": "leonas",  "lang": "lt"},
        {"id": "lt-LT-OnaNeural",      "generator": "ona",     "lang": "lt"}
    ],
    "defaults": {"lt": "lt-LT-LeonasNeural", "en": "en_US-ryan"},
    "generators_enabled": {"piper": true, "leonas": true, "ona": true},
    "lt_network_deadline_ms": 2500,
    "labels": {
        "sergeant": {"declared": null, "set": {"lt": "lt-LT-LeonasNeural", "en": "en_US-ryan"}},
        "kamane":   {"declared": null, "set": {"lt": "lt-LT-OnaNeural",    "en": "en_US-amy"}}
    }
}"#;

impl Default for OrganConfig {
    fn default() -> Self {
        parse_config(DEFAULT_CONFIG_JSON).expect("embedded default config must parse")
    }
}

/// Parse + validate a full config document.
pub fn parse_config(text: &str) -> Result<OrganConfig, ConfigErr> {
    let v = json::parse(text).map_err(|e| ConfigErr(format!("JSON: {e:?}")))?;
    from_value(&v)
}

fn from_value(v: &Value) -> Result<OrganConfig, ConfigErr> {
    let wrap = |e: RegistryErr| ConfigErr(e.0);
    let registry = Registry::from_value(v.get("registry").unwrap_or(v)).map_err(wrap)?;

    // defaults: must reference admitted voices of the right language.
    let mut defaults = BTreeMap::new();
    let dv = v
        .get("defaults")
        .ok_or_else(|| ConfigErr("missing 'defaults'".into()))?;
    for (key, lang) in [("lt", Lang::Lt), ("en", Lang::En)] {
        let id = dv
            .get(key)
            .and_then(Value::as_str)
            .ok_or_else(|| ConfigErr(format!("defaults missing '{key}'")))?;
        validate_voice(&registry, id, lang)?;
        defaults.insert(lang, id.to_string());
    }

    // generators_enabled: keys must be admitted; missing key = disabled.
    let mut enabled = BTreeMap::new();
    if let Some(map) = v.get("generators_enabled").and_then(Value::as_obj) {
        for (id, on) in map {
            if registry.generator(id).is_none() {
                return Err(ConfigErr(format!(
                    "generators_enabled: unknown generator '{id}'"
                )));
            }
            enabled.insert(id.clone(), on.as_bool().unwrap_or(false));
        }
    }

    // labels: voice sets must reference admitted voices; declared is L0.
    let mut labels = BTreeMap::new();
    if let Some(map) = v.get("labels").and_then(Value::as_obj) {
        for (name, lv) in map {
            let declared = match lv.get("declared") {
                None | Some(Value::Null) => None,
                Some(s) => Some(
                    s.as_str()
                        .unwrap_or("")
                        .parse::<Lang>()
                        .map_err(|_| ConfigErr(format!("label '{name}': bad 'declared'")))?,
                ),
            };
            let sv = lv
                .get("set")
                .ok_or_else(|| ConfigErr(format!("label '{name}': missing 'set'")))?;
            let pick = |k: &str, lang: Lang| -> Result<Option<String>, ConfigErr> {
                match sv.get(k) {
                    None | Some(Value::Null) => Ok(None),
                    Some(x) => {
                        let id = x.as_str().ok_or_else(|| {
                            ConfigErr(format!("label '{name}': set.{k} not a string"))
                        })?;
                        validate_voice(&registry, id, lang)?;
                        Ok(Some(id.to_string()))
                    }
                }
            };
            labels.insert(
                name.clone(),
                LabelConfig {
                    declared,
                    set: VoiceSet {
                        lt: pick("lt", Lang::Lt)?,
                        en: pick("en", Lang::En)?,
                    },
                },
            );
        }
    }

    let lt_network_deadline_ms = v
        .get("lt_network_deadline_ms")
        .and_then(Value::as_f64)
        .map(|n| n as u32)
        .unwrap_or(DEFAULT_LT_NETWORK_DEADLINE_MS);

    Ok(OrganConfig {
        registry,
        defaults,
        labels,
        generators_enabled: enabled,
        lt_network_deadline_ms,
    })
}

fn validate_voice(reg: &Registry, id: &str, lang: Lang) -> Result<(), ConfigErr> {
    match reg.voice(id) {
        Some(v) if v.lang == lang => Ok(()),
        Some(v) => Err(ConfigErr(format!(
            "voice '{id}' is {:?}, expected {lang:?}",
            v.lang
        ))),
        None => Err(ConfigErr(format!(
            "voice '{id}' is not in the admitted registry"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_default_config_parses_and_validates() {
        let c = OrganConfig::default();
        assert_eq!(c.registry.generators.len(), 3);
        assert_eq!(c.registry.voices.len(), 4);
        assert_eq!(c.defaults.get(&Lang::Lt).unwrap(), "lt-LT-LeonasNeural");
        assert_eq!(c.defaults.get(&Lang::En).unwrap(), "en_US-ryan");
        assert_eq!(c.lt_network_deadline_ms, 2500);
        let sgt = c.labels.get("sergeant").unwrap();
        assert_eq!(sgt.set.en.as_deref(), Some("en_US-ryan"));
        assert_eq!(sgt.declared, None);
        // Labels reference real voices of the right language (validated).
        let kam = c.labels.get("kamane").unwrap();
        assert_eq!(kam.set.lt.as_deref(), Some("lt-LT-OnaNeural"));
    }

    #[test]
    fn registry_can_nest_under_registry_key() {
        // A config that nests the registry under "registry" while keeping
        // defaults/labels at the top level parses identically (the file
        // format of slice (b) wraps the registry section this way).
        let v = json::parse(DEFAULT_CONFIG_JSON).unwrap();
        let mut pairs: Vec<(String, Value)> = vec![("registry".to_string(), v.clone())];
        if let Value::Obj(src) = &v {
            for (k, val) in src {
                if k != "generators" && k != "voices" {
                    pairs.push((k.clone(), val.clone()));
                }
            }
        }
        let c = from_value(&Value::Obj(pairs)).unwrap();
        assert_eq!(c.registry.generators.len(), 3);
        assert_eq!(c.registry.voices.len(), 4);
        assert_eq!(c.defaults.get(&Lang::En).unwrap(), "en_US-ryan");
        assert!(c.labels.contains_key("sergeant"));
    }

    #[test]
    fn wrong_language_default_voice_rejects() {
        let bad = DEFAULT_CONFIG_JSON.replace(
            r#""defaults": {"lt": "lt-LT-LeonasNeural", "en": "en_US-ryan"}"#,
            r#""defaults": {"lt": "en_US-ryan", "en": "en_US-ryan"}"#,
        );
        let e = parse_config(&bad).unwrap_err();
        assert!(e.0.contains("en_US-ryan"), "{e}");
    }

    #[test]
    fn unknown_generator_enable_flag_rejects() {
        let bad = DEFAULT_CONFIG_JSON.replace(
            r#""generators_enabled": {"piper": true, "leonas": true, "ona": true}"#,
            r#""generators_enabled": {"piper": true, "mystery": true}"#,
        );
        let e = parse_config(&bad).unwrap_err();
        assert!(e.0.contains("mystery"), "{e}");
    }

    #[test]
    fn label_with_phantom_voice_rejects() {
        let bad = DEFAULT_CONFIG_JSON.replace(r#""en": "en_US-amy""#, r#""en": "en_US-ghost""#);
        assert!(parse_config(&bad).unwrap_err().0.contains("ghost"));
    }

    #[test]
    fn declared_l0_label_parses() {
        let doc = DEFAULT_CONFIG_JSON.replace(
            r#""sergeant": {"declared": null"#,
            r#""sergeant": {"declared": "lt""#,
        );
        let c = parse_config(&doc).unwrap();
        assert_eq!(c.labels.get("sergeant").unwrap().declared, Some(Lang::Lt));
    }

    #[test]
    fn deadline_defaults_when_missing() {
        let doc = DEFAULT_CONFIG_JSON.replace(",\n    \"lt_network_deadline_ms\": 2500,", ",");
        let c = parse_config(&doc).unwrap();
        assert_eq!(c.lt_network_deadline_ms, DEFAULT_LT_NETWORK_DEADLINE_MS);
    }
}
