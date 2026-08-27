//! earcons.rs — the organ's earcon SET as validated data (verdict P2 line:
//! "drop-ledger + earcon set grows the distinct SUBSTITUTED warning
//! earcon" — R-B general-speech path).
//!
//! The four proven daemon motifs (attention / start / done / fail) are
//! ported VERBATIM from `peluda-voice/earcon_params.json` 1.0.0 — the
//! operator's ear already knows them; changing a proven sound would be a
//! regression dressed as progress. Two NEW organ-native motifs complete
//! the R-B split-by-path vocabulary:
//!
//! - `substituted` — general speech was spoken by a SUBSTITUTE voice:
//!   rising, wide, deliberately softer than `fail`. "Wrong, but working."
//! - `degrade` — a gated confirm honestly degraded to silence: quiet,
//!   slow, hollow. "Nothing came" — absence, not an alarm.
//!
//! WAV synthesis is P3 (gramophone: additive synthesis per the generator
//! notes below); this module fixes the SET and its mechanical distinctness
//! law so the synthesizer cannot ship two interchangeable warnings.

use crate::json::{self, Value};
use std::collections::BTreeMap;

/// The earcon classes the R-B verdict requires to be audibly DISTINCT —
/// an operator must tell "spoke wrong" from "spoke nothing" from "failed"
/// without looking at a screen.
pub const WARNING_CLASS: [&str; 3] = ["fail", "substituted", "degrade"];

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Stereo {
    MonoDup,
    Width(f64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct Motif {
    pub fundamental_hz: f64,
    /// (harmonic multiplier, amplitude) pairs.
    pub harmonics: Vec<(f64, f64)>,
    pub chirp_from_hz: f64,
    pub chirp_to_hz: f64,
    /// "up" | "down"
    pub chirp_direction: String,
    pub chirp_duration_ms: u32,
    pub attack_ms: u32,
    pub decay_tau_s: f64,
    pub total_duration_s: f64,
    pub stereo: Stereo,
    pub peak_dbfs: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EarconSet {
    pub version: String,
    pub motifs: BTreeMap<String, Motif>,
    /// event name → motif id.
    pub event_map: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct EarconErr(pub String);

impl std::fmt::Display for EarconErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "earcons: {}", self.0)
    }
}

/// The full set as an embedded JSON document (the same shape the daemon's
/// `earcon_params.json` uses, so the P3 synthesizer reads one grammar).
pub const EARCON_SET_JSON: &str = r#"{
    "version": "1.1.0-organ",
    "sample_rate_hz": 48000,
    "generator_notes": "Additive: sum(amp_i * sin(2*pi * f0 * mult_i * t + phase)) with linear chirp on f0 over chirp.duration_ms, then exp decay e^(-t/decay_tau_s) after attack linear ramp. Normalize to peak_dbfs.",
    "motifs": {
        "attention": {
            "fundamental_hz": 494.0,
            "harmonics": [
                {"mult": 1.0, "amp": 1.0},
                {"mult": 2.0, "amp": 0.42},
                {"mult": 3.0, "amp": 0.18},
                {"mult": 4.0, "amp": 0.08}
            ],
            "chirp": {"from_hz": 440.0, "to_hz": 698.0, "direction": "up", "duration_ms": 420},
            "attack_ms": 28,
            "decay_tau_s": 0.38,
            "total_duration_s": 1.15,
            "stereo": {"mode": "width", "width": 0.22},
            "peak_dbfs": -3,
            "used_by": ["attention", "blocker"],
            "rationale": "Rising mid-high summon: clear across a room, urgent without a startle spike."
        },
        "start": {
            "fundamental_hz": 587.0,
            "harmonics": [
                {"mult": 1.0, "amp": 1.0},
                {"mult": 2.0, "amp": 0.35},
                {"mult": 3.0, "amp": 0.12}
            ],
            "chirp": {"from_hz": 523.0, "to_hz": 740.0, "direction": "up", "duration_ms": 160},
            "attack_ms": 12,
            "decay_tau_s": 0.18,
            "total_duration_s": 0.48,
            "stereo": "mono-dup",
            "peak_dbfs": -3,
            "used_by": ["bee.launch", "quorum.start", "council.start", "daemon.start"],
            "rationale": "Short bright up-blip: something just began; learnable and non-intrusive at high rate."
        },
        "done": {
            "fundamental_hz": 392.0,
            "harmonics": [
                {"mult": 1.0, "amp": 1.0},
                {"mult": 2.0, "amp": 0.48},
                {"mult": 3.0, "amp": 0.22},
                {"mult": 5.0, "amp": 0.09}
            ],
            "chirp": {"from_hz": 523.0, "to_hz": 330.0, "direction": "down", "duration_ms": 520},
            "attack_ms": 35,
            "decay_tau_s": 0.55,
            "total_duration_s": 1.38,
            "stereo": {"mode": "width", "width": 0.28},
            "peak_dbfs": -3,
            "used_by": ["bee.done", "quorum.done", "council.done", "merge.done", "done"],
            "rationale": "Warm falling settle with longer ring: finished/resolved, opposite of the rising summons."
        },
        "fail": {
            "fundamental_hz": 196.0,
            "harmonics": [
                {"mult": 1.0, "amp": 1.0},
                {"mult": 1.059, "amp": 0.92},
                {"mult": 2.0, "amp": 0.28},
                {"mult": 2.118, "amp": 0.22},
                {"mult": 3.0, "amp": 0.1}
            ],
            "chirp": {"from_hz": 220.0, "to_hz": 175.0, "direction": "down", "duration_ms": 380},
            "attack_ms": 45,
            "decay_tau_s": 0.42,
            "total_duration_s": 1.05,
            "stereo": "mono-dup",
            "peak_dbfs": -3,
            "used_by": ["bee.fail"],
            "rationale": "Low minor-second pair (mult 1.059 = semitone) + soft down-glide: finished-but-bad, never a pleasant bell."
        },
        "substituted": {
            "fundamental_hz": 330.0,
            "harmonics": [
                {"mult": 1.0, "amp": 1.0},
                {"mult": 2.0, "amp": 0.3},
                {"mult": 2.993, "amp": 0.25}
            ],
            "chirp": {"from_hz": 277.0, "to_hz": 466.0, "direction": "up", "duration_ms": 450},
            "attack_ms": 20,
            "decay_tau_s": 0.3,
            "total_duration_s": 0.85,
            "stereo": {"mode": "width", "width": 0.5},
            "peak_dbfs": -6,
            "used_by": ["substitute"],
            "rationale": "R-B substituted warning: rising 'wrong-but-working' ask (detuned third mult 2.993), wide image, softer than fail — a caveat, not an alarm."
        },
        "degrade": {
            "fundamental_hz": 261.63,
            "harmonics": [
                {"mult": 1.0, "amp": 1.0},
                {"mult": 3.0, "amp": 0.12}
            ],
            "chirp": {"from_hz": 261.63, "to_hz": 247.0, "direction": "down", "duration_ms": 600},
            "attack_ms": 50,
            "decay_tau_s": 0.8,
            "total_duration_s": 1.6,
            "stereo": "mono-dup",
            "peak_dbfs": -8,
            "used_by": ["degrade"],
            "rationale": "R-B honest degrade: quiet hollow octave-minus-third, slow shallow fall, long fade — 'nothing came', absence rather than scolding."
        }
    },
    "event_chime_map": {
        "attention": "attention",
        "blocker": "attention",
        "bee.launch": "start",
        "quorum.start": "start",
        "council.start": "start",
        "daemon.start": "start",
        "bee.done": "done",
        "quorum.done": "done",
        "council.done": "done",
        "merge.done": "done",
        "done": "done",
        "bee.fail": "fail",
        "daemon.muted": "done",
        "substitute": "substituted",
        "degrade": "degrade"
    }
}"#;

impl Default for EarconSet {
    fn default() -> Self {
        parse_set(EARCON_SET_JSON).expect("embedded earcon set must parse")
    }
}

/// Parse + validate a set document. Fail-closed: a set is either fully
/// valid or rejected with a reason.
pub fn parse_set(text: &str) -> Result<EarconSet, EarconErr> {
    let v = json::parse(text).map_err(|e| EarconErr(format!("JSON: {e:?}")))?;
    let version = v
        .get("version")
        .and_then(Value::as_str)
        .ok_or_else(|| EarconErr("missing version".into()))?
        .to_string();
    let motifs_v = v
        .get("motifs")
        .and_then(Value::as_obj)
        .ok_or_else(|| EarconErr("missing motifs".into()))?;
    let mut motifs = BTreeMap::new();
    for (id, mv) in motifs_v {
        motifs.insert(id.clone(), parse_motif(id, mv)?);
    }
    let map_v = v
        .get("event_chime_map")
        .and_then(Value::as_obj)
        .ok_or_else(|| EarconErr("missing event_chime_map".into()))?;
    let mut event_map = BTreeMap::new();
    for (ev, target) in map_v {
        let t = target
            .as_str()
            .ok_or_else(|| EarconErr(format!("event {ev}: target not a string")))?;
        if !motifs.contains_key(t) {
            return Err(EarconErr(format!("event {ev} points at unknown motif {t}")));
        }
        event_map.insert(ev.clone(), t.to_string());
    }

    // Required vocabulary: the four proven classes + both R-B classes.
    for m in [
        "attention",
        "start",
        "done",
        "fail",
        "substituted",
        "degrade",
    ] {
        if !motifs.contains_key(m) {
            return Err(EarconErr(format!("required motif missing: {m}")));
        }
    }
    for ev in ["substitute", "degrade", "bee.fail"] {
        if !event_map.contains_key(ev) {
            return Err(EarconErr(format!("required event missing from map: {ev}")));
        }
    }
    // R-B distinctness law: every warning-class pair differs by >= 50 Hz
    // fundamental OR by chirp direction (mechanical, not aural judgment).
    let mut warn: Vec<(&str, &Motif)> = WARNING_CLASS
        .iter()
        .map(|id| (*id, motifs.get(*id).expect("checked above")))
        .collect();
    warn.sort_by_key(|(id, _)| *id);
    for i in 0..warn.len() {
        for j in i + 1..warn.len() {
            let (ai, am) = &warn[i];
            let (bi, bm) = &warn[j];
            let hz = (am.fundamental_hz - bm.fundamental_hz).abs();
            let dir = am.chirp_direction != bm.chirp_direction;
            if hz < 50.0 && !dir {
                return Err(EarconErr(format!(
                    "warning motifs {ai}/{bi} are not distinct enough ({hz:.1}Hz, same chirp direction)"
                )));
            }
        }
    }
    Ok(EarconSet {
        version,
        motifs,
        event_map,
    })
}

fn parse_motif(id: &str, v: &Value) -> Result<Motif, EarconErr> {
    let f = |k: &str| -> Result<f64, EarconErr> {
        v.get(k)
            .and_then(Value::as_f64)
            .ok_or_else(|| EarconErr(format!("motif {id}: missing number {k}")))
    };
    let u = |k: &str| -> Result<u32, EarconErr> {
        let n = f(k)?;
        if n < 0.0 || n.fract() != 0.0 {
            return Err(EarconErr(format!(
                "motif {id}: {k} must be a non-negative integer"
            )));
        }
        Ok(n as u32)
    };
    let chirp = v
        .get("chirp")
        .ok_or_else(|| EarconErr(format!("motif {id}: missing chirp")))?;
    let harmonics_v = v
        .get("harmonics")
        .and_then(Value::as_arr)
        .ok_or_else(|| EarconErr(format!("motif {id}: missing harmonics")))?;
    if harmonics_v.is_empty() {
        return Err(EarconErr(format!("motif {id}: empty harmonics")));
    }
    let mut harmonics = Vec::with_capacity(harmonics_v.len());
    for h in harmonics_v {
        let mult = h.get("mult").and_then(Value::as_f64).unwrap_or(0.0);
        let amp = h.get("amp").and_then(Value::as_f64).unwrap_or(0.0);
        if mult <= 0.0 || amp <= 0.0 {
            return Err(EarconErr(format!(
                "motif {id}: harmonic mult/amp must be positive"
            )));
        }
        harmonics.push((mult, amp));
    }
    let stereo = match v.get("stereo") {
        Some(Value::Str(s)) if s == "mono-dup" => Stereo::MonoDup,
        Some(Value::Obj(_)) => {
            let w = v
                .get("stereo")
                .and_then(|s| s.get("width"))
                .and_then(Value::as_f64)
                .ok_or_else(|| EarconErr(format!("motif {id}: stereo width missing")))?;
            if !(0.0..=1.0).contains(&w) {
                return Err(EarconErr(format!(
                    "motif {id}: stereo width {w} out of 0..1"
                )));
            }
            Stereo::Width(w)
        }
        _ => return Err(EarconErr(format!("motif {id}: bad stereo shape"))),
    };
    let m = Motif {
        fundamental_hz: f("fundamental_hz")?,
        harmonics,
        chirp_from_hz: chirp.get("from_hz").and_then(Value::as_f64).unwrap_or(0.0),
        chirp_to_hz: chirp.get("to_hz").and_then(Value::as_f64).unwrap_or(0.0),
        chirp_direction: chirp
            .get("direction")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        chirp_duration_ms: {
            let d = chirp
                .get("duration_ms")
                .and_then(Value::as_f64)
                .ok_or_else(|| EarconErr(format!("motif {id}: chirp duration_ms missing")))?;
            if d < 0.0 || d.fract() != 0.0 {
                return Err(EarconErr(format!(
                    "motif {id}: chirp duration_ms must be a non-negative integer"
                )));
            }
            d as u32
        },
        attack_ms: u("attack_ms")?,
        decay_tau_s: f("decay_tau_s")?,
        total_duration_s: f("total_duration_s")?,
        stereo,
        peak_dbfs: f("peak_dbfs")?,
    };
    if !matches!(m.chirp_direction.as_str(), "up" | "down") {
        return Err(EarconErr(format!(
            "motif {id}: chirp direction must be up/down"
        )));
    }
    if !(0.1..=3.0).contains(&m.total_duration_s) {
        return Err(EarconErr(format!(
            "motif {id}: duration {} out of 0.1..=3.0s",
            m.total_duration_s
        )));
    }
    if m.attack_ms == 0 || m.attack_ms > 100 {
        return Err(EarconErr(format!(
            "motif {id}: attack {} out of 1..=100ms",
            m.attack_ms
        )));
    }
    if m.decay_tau_s <= 0.0 {
        return Err(EarconErr(format!("motif {id}: decay tau must be positive")));
    }
    if f64::from(m.chirp_duration_ms) / 1000.0 > m.total_duration_s {
        return Err(EarconErr(format!(
            "motif {id}: chirp longer than the motif"
        )));
    }
    if !(-24.0..=-1.0).contains(&m.peak_dbfs) {
        return Err(EarconErr(format!(
            "motif {id}: peak_dbfs {} out of -24..=-1",
            m.peak_dbfs
        )));
    }
    Ok(m)
}

impl EarconSet {
    /// The motif an event maps to (None = the event has no earcon).
    pub fn earcon_for(&self, event: &str) -> Option<&Motif> {
        self.event_map.get(event).and_then(|id| self.motifs.get(id))
    }

    /// The R-B verdict's earcon choices, named: what fires on a
    /// substitution and what fires on an honest degrade.
    pub fn substituted_warning(&self) -> &Motif {
        self.motifs.get("substituted").expect("validated at parse")
    }

    pub fn degrade_chime(&self) -> &Motif {
        self.motifs.get("degrade").expect("validated at parse")
    }
}

// ---------------------------------------------------------------------------
// P3 synthesis — the generator notes, made code
// ---------------------------------------------------------------------------

/// The set's declared sample rate (`sample_rate_hz`, 48 kHz — the daemon's
/// earcon family rate; the play child's resampler adapts to any device).
pub const EARCON_SAMPLE_RATE: u32 = 48_000;

/// Synthesize one motif into a PCM16 stereo WAV, exactly per the set's
/// `generator_notes`:
///
/// `sum(amp_i * sin(2π f0 mult_i t + phase))` with a LINEAR chirp on `f0`
/// over `chirp.duration_ms`, an attack linear ramp, an `e^(-t/decay_tau_s)`
/// decay, normalized to `peak_dbfs`.
///
/// Phase is accumulated (`phi += 2π f0/sr`) rather than computed as
/// `2π f(t) t` — a chirp's phase is the INTEGRAL of instantaneous
/// frequency; the closed form's `f·t` term audibly detunes the sweep.
/// `fundamental_hz` is the nominal pitch for the distinctness law; the
/// sweep itself runs `chirp_from_hz → chirp_to_hz` and holds at the end
/// value for the remainder of the motif (the daemon's generator behavior).
///
/// Stereo: `mono-dup` duplicates; `width w` pans the same signal
/// `1 − w/2` / `1 + w/2` — an IMAGE, not a decorrelated pair; the
/// warning set's "wide" substitution cue is width, not phase games.
/// Normalization happens AFTER stereo shaping so the LOUDER channel hits
/// `peak_dbfs` exactly.
pub fn synth_wav(motif: &Motif, sample_rate: u32) -> Vec<u8> {
    let sr = f64::from(sample_rate);
    let n = (motif.total_duration_s * sr).round() as usize;
    let chirp_s = f64::from(motif.chirp_duration_ms) / 1000.0;
    let attack_s = f64::from(motif.attack_ms) / 1000.0;
    let (lg, rg) = match motif.stereo {
        Stereo::MonoDup => (1.0, 1.0),
        Stereo::Width(w) => (1.0 - 0.5 * w, 1.0 + 0.5 * w),
    };

    let mut left: Vec<f64> = Vec::with_capacity(n);
    let mut right: Vec<f64> = Vec::with_capacity(n);
    let mut phi = 0.0_f64; // accumulated fundamental phase, radians
    let mut peak = 0.0_f64;
    for i in 0..n {
        let t = i as f64 / sr;
        let f0 = if chirp_s > 0.0 && t < chirp_s {
            motif.chirp_from_hz + (motif.chirp_to_hz - motif.chirp_from_hz) * (t / chirp_s)
        } else {
            motif.chirp_to_hz
        };
        phi += 2.0 * std::f64::consts::PI * f0 / sr;
        // Attack ramps 0→1; decay runs from t=0 (the daemon multiplied both).
        let env = (if t < attack_s { t / attack_s } else { 1.0 })
            * (-t / motif.decay_tau_s).exp();
        let mut s = 0.0;
        for (mult, amp) in &motif.harmonics {
            s += amp * (mult * phi).sin();
        }
        let (l, r) = (s * env * lg, s * env * rg);
        peak = peak.max(l.abs()).max(r.abs());
        left.push(l);
        right.push(r);
    }

    // Peak-normalize so the loudest SAMPLE sits exactly at peak_dbfs.
    let scale = if peak > 0.0 {
        10.0_f64.powf(motif.peak_dbfs / 20.0) / peak
    } else {
        0.0
    };
    let to_i16 = |x: f64| -> i16 {
        (x * scale * 32767.0).round().clamp(-32767.0, 32767.0) as i16
    };

    let mut pcm: Vec<u8> = Vec::with_capacity(n * 4);
    for (l, r) in left.iter().zip(right.iter()) {
        pcm.extend_from_slice(&to_i16(*l).to_le_bytes());
        pcm.extend_from_slice(&to_i16(*r).to_le_bytes());
    }
    write_wav_pcm16(&pcm, 2, sample_rate)
}

/// Canonical 44-byte RIFF/WAVE header + PCM16 payload (the byte layout
/// `transcribe::wav_meta` and `play::read_wav` both walk).
fn write_wav_pcm16(data: &[u8], channels: u16, sample_rate: u32) -> Vec<u8> {
    let byte_rate = sample_rate * u32::from(channels) * 2;
    let block_align = channels * 2;
    let mut wav = Vec::with_capacity(44 + data.len());
    wav.extend_from_slice(b"RIFF");
    wav.extend_from_slice(&((36 + data.len()) as u32).to_le_bytes());
    wav.extend_from_slice(b"WAVEfmt ");
    wav.extend_from_slice(&16u32.to_le_bytes()); // fmt chunk size
    wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
    wav.extend_from_slice(&channels.to_le_bytes());
    wav.extend_from_slice(&sample_rate.to_le_bytes());
    wav.extend_from_slice(&byte_rate.to_le_bytes());
    wav.extend_from_slice(&block_align.to_le_bytes());
    wav.extend_from_slice(&16u16.to_le_bytes()); // bits per sample
    wav.extend_from_slice(b"data");
    wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
    wav.extend_from_slice(data);
    wav
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_set_parses_with_all_events() {
        let set = EarconSet::default();
        assert_eq!(set.version, "1.1.0-organ");
        assert_eq!(set.motifs.len(), 6);
        assert!(set.earcon_for("attention").is_some());
        assert!(set.earcon_for("merge.done").is_some());
        // The R-B verdict events resolve to the DISTINCT new motifs.
        let sub = set.earcon_for("substitute").unwrap();
        assert_eq!(sub.fundamental_hz, 330.0);
        assert!(matches!(sub.stereo, Stereo::Width(0.5)));
        let deg = set.earcon_for("degrade").unwrap();
        assert_eq!(deg.fundamental_hz, 261.63);
        assert_eq!(set.earcon_for("unknown.event"), None);
        assert_eq!(set.substituted_warning().chirp_direction, "up");
        assert_eq!(set.degrade_chime().peak_dbfs, -8.0);
    }

    #[test]
    fn ported_motifs_are_verbatim_daemon_values() {
        let set = EarconSet::default();
        let fail = set.motifs.get("fail").unwrap();
        assert_eq!(fail.fundamental_hz, 196.0);
        assert_eq!(fail.harmonics.len(), 5);
        assert_eq!(fail.harmonics[1], (1.059, 0.92));
        assert_eq!(fail.chirp_from_hz, 220.0);
        assert_eq!(fail.chirp_to_hz, 175.0);
        assert_eq!(fail.chirp_duration_ms, 380);
        assert_eq!(fail.attack_ms, 45);
        assert_eq!(set.motifs.get("attention").unwrap().total_duration_s, 1.15);
        assert!(matches!(
            set.motifs.get("start").unwrap().stereo,
            Stereo::MonoDup
        ));
    }

    #[test]
    fn warning_class_distinctness_law_rejects_clones() {
        // substituted mutated to fail's signature → the set must be refused.
        let bad = EARCON_SET_JSON.replacen(
            r#""fundamental_hz": 330.0,
            "harmonics": [
                {"mult": 1.0, "amp": 1.0},
                {"mult": 2.0, "amp": 0.3},
                {"mult": 2.993, "amp": 0.25}
            ],
            "chirp": {"from_hz": 277.0, "to_hz": 466.0, "direction": "up", "duration_ms": 450}"#,
            r#""fundamental_hz": 200.0,
            "harmonics": [
                {"mult": 1.0, "amp": 1.0},
                {"mult": 2.0, "amp": 0.3},
                {"mult": 2.993, "amp": 0.25}
            ],
            "chirp": {"from_hz": 220.0, "to_hz": 175.0, "direction": "down", "duration_ms": 380}"#,
            1,
        );
        assert_ne!(&bad, EARCON_SET_JSON, "mutation must land");
        let e = parse_set(&bad).unwrap_err();
        assert!(e.0.contains("not distinct enough"), "unexpected err: {e}");
    }

    #[test]
    fn parse_rejects_missing_and_malformed() {
        assert!(parse_set("{}").is_err());
        // Missing a required motif.
        let no_sub = EARCON_SET_JSON.replace("\"substituted\": {", "\"renamed_substituted\": {");
        assert!(parse_set(&no_sub).is_err());
        // Event pointing at an unknown motif.
        let bad_map = EARCON_SET_JSON.replacen(
            "\"substitute\": \"substituted\"",
            "\"substitute\": \"nonexistent\"",
            1,
        );
        assert!(parse_set(&bad_map).is_err());
        // Out-of-bounds duration.
        let bad_dur = EARCON_SET_JSON.replacen(
            "\"total_duration_s\": 1.15",
            "\"total_duration_s\": 30.0",
            1,
        );
        assert!(parse_set(&bad_dur).is_err());
    }

    // ---------------- synthesis (P3 slice c) ----------------

    use crate::transcribe::wav_meta;

    /// Decode a synth WAV into interleaved i16 frames.
    fn frames(wav: &[u8]) -> Vec<i16> {
        wav[44..]
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect()
    }

    fn mono(wav: &[u8]) -> Vec<f64> {
        frames(wav)
            .chunks_exact(2)
            .map(|p| (f64::from(p[0]) + f64::from(p[1])) / 2.0)
            .collect()
    }

    #[test]
    fn synth_matches_declared_shape() {
        let set = EarconSet::default();
        let m = set.motifs.get("attention").unwrap();
        let wav = synth_wav(m, EARCON_SAMPLE_RATE);
        let meta = wav_meta(&wav).expect("synth output must be a valid WAV");
        assert_eq!(meta.sample_rate, EARCON_SAMPLE_RATE);
        assert_eq!(meta.channels, 2);
        assert!(
            (meta.duration_s - m.total_duration_s).abs() < 0.005,
            "duration {} vs {}",
            meta.duration_s,
            m.total_duration_s
        );
    }

    #[test]
    fn synth_normalizes_peak_to_dbfs_and_ramps_in() {
        let set = EarconSet::default();
        for id in ["attention", "substituted", "degrade"] {
            let m = set.motifs.get(id).unwrap();
            let wav = synth_wav(m, EARCON_SAMPLE_RATE);
            let fr = frames(&wav);
            let peak = fr.iter().map(|s| s.unsigned_abs()).max().unwrap() as f64;
            let target = 32767.0 * 10.0_f64.powf(m.peak_dbfs / 20.0);
            assert!(
                (peak - target).abs() / target < 0.01,
                "{id}: peak {peak} vs target {target}"
            );
            // Attack ramp: the first frame is inside 1% of silence.
            let first = fr.iter().take(4).map(|s| s.unsigned_abs()).max().unwrap() as f64;
            assert!(
                (first < target * 0.01),
                "{id}: attack must start at (near) zero"
            );
        }
    }

    #[test]
    fn synth_stereo_law_is_exactly_the_shape() {
        let set = EarconSet::default();
        // mono-dup: identical channels everywhere.
        let dup = synth_wav(set.motifs.get("start").unwrap(), EARCON_SAMPLE_RATE);
        for pair in frames(&dup).chunks_exact(2) {
            assert_eq!(pair[0], pair[1]);
        }
        // width 0.5: channels differ; the RIGHT channel is the louder one.
        let wide = synth_wav(set.substituted_warning(), EARCON_SAMPLE_RATE);
        let fr = frames(&wide);
        let li = fr
            .chunks_exact(2)
            .enumerate()
            .max_by_key(|(_, p)| p[0].unsigned_abs() + p[1].unsigned_abs())
            .map(|(i, _)| i)
            .unwrap();
        assert_ne!(fr[li * 2], fr[li * 2 + 1], "width motif must be stereo");
        assert!(
            fr[li * 2 + 1].unsigned_abs() > fr[li * 2].unsigned_abs(),
            "width 0.5 pans RIGHT-loud"
        );
    }

    #[test]
    fn synth_warning_motifs_are_audibly_distinct() {
        // The parse-time law checks PARAMETERS; the synth-level law checks
        // the actual WAVEFORMS: normalized cross-correlation of the mono
        // mixes of every warning pair must stay low — the operator cannot
        // mistake one for another when they differ this much in shape.
        let set = EarconSet::default();
        let wavs: Vec<(&str, Vec<f64>)> = WARNING_CLASS
            .iter()
            .map(|id| {
                (
                    *id,
                    mono(&synth_wav(set.motifs.get(*id).unwrap(), EARCON_SAMPLE_RATE)),
                )
            })
            .collect();
        for i in 0..wavs.len() {
            for j in i + 1..wavs.len() {
                let (ai, av) = &wavs[i];
                let (bi, bv) = &wavs[j];
                let n = av.len().min(bv.len());
                let (sa, sb) = (&av[..n], &bv[..n]);
                let dot: f64 = sa.iter().zip(sb).map(|(x, y)| x * y).sum();
                let na: f64 = sa.iter().map(|x| x * x).sum::<f64>().sqrt();
                let nb: f64 = sb.iter().map(|x| x * x).sum::<f64>().sqrt();
                let ncc = dot / (na * nb);
                assert!(
                    ncc.abs() < 0.5,
                    "{ai}/{bi} waveforms too similar: ncc={ncc:.3}"
                );
            }
        }
    }
}
