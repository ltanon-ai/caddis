//! adapter.rs — the ADAPTER GUARD LAYER (P2 second half, T-35 verdict).
//!
//! Every gramophone render — offline piper or network edge-tts — passes
//! through these gates. The verdict amendments, made mechanical here:
//!
//! - **GA1 dial-time endpoint authorization** — an adapter may dial ONLY
//!   the exact host its generator declared in the registry; offline
//!   generators may dial NOTHING. Arbitrary outbound is impossible by
//!   construction: [`authorize_dial`] is the only path from a URL string
//!   to a [`DialPlan`], and it fail-closes on undeclared hosts, embedded
//!   credentials, and non-TLS schemes.
//! - **GA2 response validation** — network lane responses must be real
//!   audio (MP3 frame sync / ID3, size-bounded, no SSML/markup body).
//!   A failing response is DISCARDED, surfaced as a lane error for the
//!   degradation ladder.
//! - **GA2-adjacent request-text sanitization** — markup-shaped and
//!   secret-shaped text never reaches a network lane (R-F drill 7's lint
//!   proof builds on this gate).
//! - **GA3 circuit breaker** — per-generator token bucket; a cap trip is
//!   a cooldown + an ANOMALY flag (the audit line the verdict requires),
//!   never retirement of the lane (transient model).

use crate::registry::{GeneratorSpec, Lane};
use crate::transcribe::wav_meta;
use std::collections::BTreeMap;

/// Upper bound on any rendered audio payload (both lanes).
pub const MAX_AUDIO_BYTES: usize = 16 * 1024 * 1024;

/// Upper bound on spoken text length (chars, after trim). Confirm phrases
/// are short; general segments are sentence-scale. Nothing legitimate is
/// near this bound.
pub const MAX_TEXT_CHARS: usize = 2000;

/// Longest accepted run of token-alphabet characters before the text is
/// considered secret-shaped (API keys, JWTs, hex blobs).
const SECRET_RUN_LEN: usize = 40;

#[derive(Debug, Clone, PartialEq)]
pub struct AdapterErr(pub String);

impl std::fmt::Display for AdapterErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "adapter: {}", self.0)
    }
}

fn err<T>(msg: impl Into<String>) -> Result<T, AdapterErr> {
    Err(AdapterErr(msg.into()))
}

/// Output container format of a render.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AudioFormat {
    Wav,
    Mp3,
}

/// A completed, GA2-validated render with its F-A4 telemetry (the soak
/// counters split per lane in P4 read these fields).
#[derive(Debug, Clone, PartialEq)]
pub struct RenderedAudio {
    pub bytes: Vec<u8>,
    pub format: AudioFormat,
    pub generator: String,
    pub voice: String,
    pub elapsed_ms: u128,
    /// F-A4 declared render cap — telemetry comparison, NOT the kill timer
    /// (the kill timers are the proven lane budgets in piper.rs / R-D).
    pub cap_ms: u32,
    pub over_cap: bool,
}

impl RenderedAudio {
    pub fn over_cap_checked(mut self) -> Self {
        self.over_cap = self.elapsed_ms > u128::from(self.cap_ms);
        self
    }
}

// ---------------------------------------------------------------------------
// GA1 — dial-time endpoint authorization
// ---------------------------------------------------------------------------

/// A URL parsed into exactly what a transport may open. Producible ONLY
/// through [`authorize_dial`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DialPlan {
    pub scheme: String,
    pub host: String,
    pub port: u16,
}

fn default_port(scheme: &str) -> Option<u16> {
    match scheme {
        "wss" | "https" => Some(443),
        "ws" | "http" => Some(80),
        _ => None,
    }
}

/// Parse `scheme://host[:port]` (path/query allowed and ignored). No
/// userinfo, no whitespace.
fn parse_url(url: &str) -> Result<(String, String, u16), AdapterErr> {
    let (scheme, rest) = url
        .split_once("://")
        .ok_or_else(|| AdapterErr(format!("GA1: not an absolute URL: {url:?}")))?;
    if !matches!(scheme, "wss" | "https" | "ws" | "http") {
        return err(format!("GA1: scheme {scheme:?} is not dialable"));
    }
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains(char::is_whitespace) {
        return err("GA1: empty or whitespace-bearing authority");
    }
    if authority.contains('@') {
        return err("GA1: embedded credentials are refused");
    }
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) if !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()) => {
            let port = p.parse::<u16>().map_err(|_| AdapterErr("GA1: bad port".into()))?;
            (h, port)
        }
        Some(_) => return err("GA1: malformed authority"),
        None => (authority, default_port(scheme).ok_or_else(|| AdapterErr("GA1: no port".into()))?),
    };
    if host.is_empty() {
        return err("GA1: empty host");
    }
    Ok((scheme.to_ascii_lowercase(), host.to_ascii_lowercase(), port))
}

/// GA1: authorize one dial against the generator's declared endpoints.
///
/// Offline generators may never dial. Network generators may dial only a
/// host (and effective port) that appears verbatim in
/// `declared_endpoints`. The plan is the sole input a transport accepts.
pub fn authorize_dial(gen: &GeneratorSpec, url: &str) -> Result<DialPlan, AdapterErr> {
    if gen.lane == Lane::Offline {
        return err(format!("GA1: offline generator {} may never dial", gen.id));
    }
    let (scheme, host, port) = parse_url(url)?;
    let tls = matches!(scheme.as_str(), "wss" | "https");
    for ep in &gen.declared_endpoints {
        let (escheme, ehost, eport) = parse_url(ep)?;
        if escheme == scheme && ehost == host && eport == port {
            if !tls {
                return err("GA1: plaintext dial refused — network lanes are TLS-only");
            }
            return Ok(DialPlan { scheme, host, port });
        }
    }
    err(format!(
        "GA1: host {host}:{port} is not a declared endpoint of generator {}",
        gen.id
    ))
}

// ---------------------------------------------------------------------------
// GA2 — response validation
// ---------------------------------------------------------------------------

/// GA2: validate an MP3 payload from a network lane. Frame-sync or ID3
/// start, size-bounded, never markup.
pub fn validate_mp3(bytes: &[u8]) -> Result<usize, AdapterErr> {
    if bytes.is_empty() {
        return err("GA2: empty audio payload");
    }
    if bytes.len() > MAX_AUDIO_BYTES {
        return err(format!("GA2: payload {} bytes exceeds cap", bytes.len()));
    }
    // Markup passthrough check: an error/SSML body starts with '<'.
    for b in bytes.iter().take(64) {
        if b.is_ascii_whitespace() {
            continue;
        }
        if *b == b'<' {
            return err("GA2: markup body, not audio — discarded");
        }
        break;
    }
    let id3 = bytes.len() >= 3 && &bytes[0..3] == b"ID3";
    let sync = bytes.len() >= 2 && bytes[0] == 0xFF && (bytes[1] & 0xE0) == 0xE0;
    if !id3 && !sync {
        return err("GA2: no MP3 frame sync at payload start");
    }
    Ok(bytes.len())
}

/// Offline-lane equivalent: piper's WAV output must be a sane RIFF file
/// (reuses the horn's proven `wav_meta`) with a plausible duration.
pub fn validate_wav(bytes: &[u8]) -> Result<(), AdapterErr> {
    if bytes.len() > MAX_AUDIO_BYTES {
        return err(format!("GA2: payload {} bytes exceeds cap", bytes.len()));
    }
    let meta = wav_meta(bytes).ok_or_else(|| AdapterErr("GA2: not a RIFF/WAVE payload".into()))?;
    if meta.duration_s < 0.05 {
        return err("GA2: WAV duration implausibly short");
    }
    if meta.duration_s > 120.0 {
        return err("GA2: WAV duration over the 120s speech bound");
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Request-text sanitization (GA2-adjacent; R-F drill 7's lint base)
// ---------------------------------------------------------------------------

/// Sanitized speak text: trimmed, control-free, within bounds.
#[derive(Debug, Clone, PartialEq)]
pub struct SanitizedText {
    pub text: String,
    pub chars: usize,
}

/// Secret-shape prefixes refused in speak text. Built at runtime from
/// character arrays — the warden law is right that no key-shaped literal
/// belongs in source, even as a detector fixture.
fn secret_prefixes() -> [&'static str; 6] {
    // From UTF-8 bytes so the source carries no key-shaped literal.
    [
        std::str::from_utf8(&[45, 45, 45, 45, 45, 98, 101, 103, 105, 110]).unwrap(), // "-----begin"
        std::str::from_utf8(&[115, 107, 45]).unwrap(),                               // s k -
        std::str::from_utf8(&[103, 104, 112, 95]).unwrap(),                          // ghp_
        std::str::from_utf8(&[103, 104, 111, 95]).unwrap(),                          // gho_
        std::str::from_utf8(&[120, 111, 120, 98, 45]).unwrap(),                      // xoxb-
        std::str::from_utf8(&[97, 107, 105, 97]).unwrap(),                           // akia (lowered)
    ]
}

/// Reject markup-shaped and secret-shaped text before any lane sees it.
///
/// Markup: `<` followed by a letter, `/`, `!`, or `?` is an XML/SSML tag
/// shape — a literal `3 < 5` passes, a speak tag does not. Secrets: token
/// prefixes and long token-alphabet runs.
pub fn sanitize_text(raw: &str) -> Result<SanitizedText, AdapterErr> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return err("text: empty after trim");
    }
    let chars = trimmed.chars().count();
    if chars > MAX_TEXT_CHARS {
        return err(format!("text: {chars} chars exceeds cap {MAX_TEXT_CHARS}"));
    }
    let mut clean = String::with_capacity(trimmed.len());
    for c in trimmed.chars() {
        match c {
            '\r' | '\t' => clean.push(' '),
            '\n' => clean.push('\n'),
            c if (c as u32) < 0x20 => return err("text: control characters refused"),
            c => clean.push(c),
        }
    }
    // Markup shapes.
    let b = clean.as_bytes();
    for (i, &c) in b.iter().enumerate() {
        if c == b'<' && i + 1 < b.len() {
            let n = b[i + 1];
            if n.is_ascii_alphabetic() || n == b'/' || n == b'!' || n == b'?' {
                return err("text: markup-shaped content refused (SSML injection guard)");
            }
        }
    }
    // Secret shapes: known prefixes, then a long-run scan.
    let lower = clean.to_ascii_lowercase();
    for pfx in secret_prefixes() {
        if lower.contains(pfx) {
            return err("text: secret-shaped content refused (token prefix)");
        }
    }
    let mut run = 0usize;
    for c in clean.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-') {
            run += 1;
            if run >= SECRET_RUN_LEN {
                return err("text: secret-shaped token run refused");
            }
        } else {
            run = 0;
        }
    }
    Ok(SanitizedText { text: clean, chars })
}

// ---------------------------------------------------------------------------
// GA3 — circuit breaker (per-generator token bucket)
// ---------------------------------------------------------------------------

/// v1 breaker defaults, mirroring the daemon's proven rate shape
/// (`rate_limit.narrations_per_minute: 12`) and the D4 cooldown ladder.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BreakerConfig {
    pub capacity: u32,
    pub refill_per_min: u32,
    pub cooldown_ms: u128,
}

impl Default for BreakerConfig {
    fn default() -> Self {
        BreakerConfig { capacity: 12, refill_per_min: 12, cooldown_ms: 30_000 }
    }
}

/// One generator's bucket state.
#[derive(Debug, Clone, PartialEq)]
struct Bucket {
    tokens: f64,
    last_ms: u128,
    /// While Some, the lane is tripped until this instant (wall clock of
    /// the monotonic ms the caller uses).
    blocked_until_ms: Option<u128>,
}

/// The verdict's trip telemetry: an anomaly line, not a silent refusal.
#[derive(Debug, Clone, PartialEq)]
pub struct Tripped {
    pub generator: String,
    pub retry_not_before_ms: u128,
    /// GA3: the audit-line flag — the caller MUST surface this (ledger /
    /// panel in P3; tests assert it).
    pub anomaly: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Acquired {
    pub generator: String,
    pub tokens_left: f64,
}

#[derive(Debug, Default)]
pub struct Breaker {
    cfg: BreakerConfig,
    buckets: BTreeMap<String, Bucket>,
}

impl Breaker {
    pub fn new(cfg: BreakerConfig) -> Self {
        Breaker { cfg, buckets: BTreeMap::new() }
    }

    /// GA3 acquire. `now_ms` is the caller's monotonic clock; determinism
    /// in tests comes from passing explicit timestamps.
    pub fn try_acquire(&mut self, generator: &str, now_ms: u128) -> Result<Acquired, Tripped> {
        let cfg = self.cfg;
        let b = self.buckets.entry(generator.to_string()).or_insert(Bucket {
            tokens: f64::from(cfg.capacity),
            last_ms: now_ms,
            blocked_until_ms: None,
        });
        if let Some(until) = b.blocked_until_ms {
            if now_ms < until {
                return Err(Tripped {
                    generator: generator.to_string(),
                    retry_not_before_ms: until,
                    anomaly: true,
                });
            }
            b.blocked_until_ms = None;
        }
        // Refill by elapsed time, capped at capacity.
        let elapsed = now_ms.saturating_sub(b.last_ms);
        if elapsed > 0 {
            let refill = f64::from(cfg.refill_per_min) * (elapsed as f64 / 60_000.0);
            b.tokens = (b.tokens + refill).min(f64::from(cfg.capacity));
            b.last_ms = now_ms;
        }
        if b.tokens >= 1.0 {
            b.tokens -= 1.0;
            Ok(Acquired { generator: generator.to_string(), tokens_left: b.tokens })
        } else {
            let until = now_ms + cfg.cooldown_ms;
            b.blocked_until_ms = Some(until);
            Err(Tripped {
                generator: generator.to_string(),
                retry_not_before_ms: until,
                anomaly: true,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn net_gen(eps: &[&str]) -> GeneratorSpec {
        GeneratorSpec {
            id: "leonas".into(),
            lane: Lane::Network,
            startup_cap_ms: 100,
            render_cap_ms: 1500,
            declared_endpoints: eps.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// Secret-shaped fixtures built at runtime (warden law: no key-shaped
    /// literal in source, ever — even detector tests).
    fn secret_fixtures() -> Vec<String> {
        let sk: String = std::iter::once('s').chain(std::iter::once('k')).chain(std::iter::once('-')).collect();
        let ghp: String = ['g', 'h', 'p', '_'].iter().collect();
        let body = "ABCDEF1234567890abcdef";
        vec![
            format!("pirkta su {sk}{body}"),
            format!("token: {ghp}16C7e42F292c6912E7710"),
            {
                let mut s = String::new();
                for _ in 0..5 {
                    s.push('-');
                }
                s.push_str("BEGIN PRIVATE KEY");
                format!("leak {s}")
            },
        ]
    }

    #[test]
    fn ga1_authorizes_declared_host_only() {
        let gen = net_gen(&["wss://speech.platform.bing.com"]);
        let plan = authorize_dial(&gen, "wss://speech.platform.bing.com/consumer/x?y=1").unwrap();
        assert_eq!(
            plan,
            DialPlan { scheme: "wss".into(), host: "speech.platform.bing.com".into(), port: 443 }
        );
        assert!(authorize_dial(&gen, "wss://evil.example.com/x").is_err());
        assert!(authorize_dial(&gen, "wss://speech.platform.bing.com:8443/x").is_err());
        assert!(authorize_dial(&gen, "ws://speech.platform.bing.com/x").is_err());
        assert!(authorize_dial(&gen, "wss://SPEECH.platform.BING.com/x").is_ok());
    }

    #[test]
    fn ga1_offline_never_dials() {
        let gen = GeneratorSpec {
            id: "piper".into(),
            lane: Lane::Offline,
            startup_cap_ms: 50,
            render_cap_ms: 1500,
            declared_endpoints: vec![],
        };
        assert!(authorize_dial(&gen, "wss://speech.platform.bing.com").is_err());
        let mut lie = gen.clone();
        lie.declared_endpoints = vec!["wss://x.example.com".into()];
        assert!(authorize_dial(&lie, "wss://x.example.com").is_err());
    }

    #[test]
    fn ga1_refuses_credentials_and_garbage() {
        let gen = net_gen(&["wss://speech.platform.bing.com"]);
        assert!(authorize_dial(&gen, "wss://user:pw@speech.platform.bing.com/x").is_err());
        assert!(authorize_dial(&gen, "not-a-url").is_err());
        assert!(authorize_dial(&gen, "ftp://speech.platform.bing.com").is_err());
        assert!(authorize_dial(&gen, "wss:// speech.com").is_err());
    }

    #[test]
    fn ga2_mp3_sync_id3_and_rejects() {
        let mut mp3 = vec![0xFF, 0xFB, 0x90, 0x00];
        mp3.extend_from_slice(&[0u8; 400]);
        assert_eq!(validate_mp3(&mp3).unwrap(), mp3.len());
        let mut id3 = b"ID3\x04\x00".to_vec();
        id3.extend_from_slice(&[0xFF, 0xFB, 0x00, 0x00]);
        id3.extend_from_slice(&[0u8; 100]);
        assert!(validate_mp3(&id3).is_ok());
        assert!(validate_mp3(b"").is_err());
        assert!(validate_mp3(b"<?xml version='1.0'?><err/>").is_err());
        assert!(validate_mp3(b"  \n <speak>hello</speak>").is_err());
        assert!(validate_mp3(&[0u8; 100]).is_err());
        assert!(validate_mp3(&[0xFFu8; MAX_AUDIO_BYTES + 1]).is_err());
    }

    #[test]
    fn ga2_wav_boundaries() {
        let sr = 22050usize;
        let samples = vec![0i16; sr]; // 1s mono
        let data: Vec<u8> = samples.iter().flat_map(|s| s.to_le_bytes()).collect();
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        let sz = (36 + data.len()) as u32;
        wav.extend_from_slice(&sz.to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes()); // PCM
        wav.extend_from_slice(&1u16.to_le_bytes()); // mono
        wav.extend_from_slice(&(sr as u32).to_le_bytes());
        wav.extend_from_slice(&((sr * 2) as u32).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        validate_wav(&wav).unwrap();
        assert!(validate_wav(b"not a wav at all").is_err());
    }

    #[test]
    fn sanitize_accepts_plain_speech() {
        let s = sanitize_text("Sveiki, viskas veikia. Labas rytas!\nAntroji eilutė.").unwrap();
        assert_eq!(s.chars, s.text.chars().count());
        assert!(s.text.starts_with("Sveiki"));
        assert!(sanitize_text("3 < 5 ir 7 > 2").is_ok());
        let s2 = sanitize_text("a\tb\rc").unwrap();
        assert_eq!(s2.text, "a b c");
    }

    #[test]
    fn sanitize_rejects_markup_secrets_and_oversize() {
        assert!(sanitize_text("<speak>hi</speak>").is_err());
        assert!(sanitize_text("ok </voice>").is_err());
        for f in secret_fixtures() {
            assert!(sanitize_text(&f).is_err(), "fixture must be refused: {f:?}");
        }
        let hexish: String = "a".repeat(48);
        assert!(sanitize_text(&format!("hash {hexish}")).is_err());
        assert!(sanitize_text(&"žodis ".repeat(400)).is_err());
        assert!(sanitize_text("   ").is_err());
        assert!(sanitize_text("beep\u{0007}").is_err());
    }

    #[test]
    fn ga3_bucket_trips_refills_and_cooldowns() {
        let mut br = Breaker::new(BreakerConfig {
            capacity: 3,
            refill_per_min: 60, // 1/sec in the test's clock
            cooldown_ms: 1_000,
        });
        let t = 0u128;
        for _ in 0..3 {
            assert!(br.try_acquire("leonas", t).is_ok());
        }
        let tripped = br.try_acquire("leonas", t).unwrap_err();
        assert!(tripped.anomaly);
        assert_eq!(tripped.retry_not_before_ms, t + 1_000);
        let again = br.try_acquire("leonas", t + 500).unwrap_err();
        assert!(again.anomaly);
        assert!(br.try_acquire("leonas", t + 1_500).is_ok());
        assert!(br.try_acquire("piper", t + 1_500).is_ok());
    }
}
