//! edgetts.rs — the DIRECT edge-tts protocol (network LT lane: leonas +
//! ona; GA-guarded per the T-35 verdict).
//!
//! This slice implements the WHOLE protocol: the DRM token (`Sec-MS-GEC`),
//! the dial URL (routed through GA1 [`authorize_dial`] — the organ dials
//! only the declared host, by construction), the two handshake messages,
//! SSML assembly with escaping, binary/text frame parsing, the R-D
//! single-attempt deadline, and GA2 MP3 validation of the accumulated
//! audio.
//!
//! The TLS/WSS TRANSPORT is deliberately the NEXT unit (schannel FFI
//! under the std-only law): [`WsStream`] below is the exact contract it
//! will implement. Until it lands, nothing in this crate dials anyone —
//! fail-closed by construction, and the mock-stream tests prove the
//! protocol end-to-end. Voice parameter defaults (`+0%`/`+0Hz`) mirror the
//! reference client's neutral defaults.

use crate::adapter::{
    authorize_dial, sanitize_text, validate_mp3, AdapterErr, AudioFormat, RenderedAudio,
};
use crate::registry::GeneratorSpec;
use crate::sha256::{sha256, sha256_hex, sha256_hex_upper};
use std::time::Instant;

/// The public edge-tts client token (shipped verbatim by every reference
/// client; an ecosystem constant, not a secret — the secrets law is about
/// OUR credentials, and nothing of ours rides this lane).
pub const TRUSTED_CLIENT_TOKEN: &str = "6A5AA1D4EAFF4E9FB37E23D68491D6F4";

/// Client version reported with the DRM token. MUST track the reference
/// client's current Chromium build — the edge rejects stale versions with
/// a plain 403 (caught live 2026-08-27: the old 130-era string was the
/// whole refusal).
pub const SEC_MS_GEC_VERSION: &str = "1-143.0.3650.75";

/// The single host the LT lanes declare (must match the registry's
/// `declared_endpoints` — GA1 authorizes only the declared host).
pub const WSS_HOST: &str = "speech.platform.bing.com";

/// Reference-client output format: MP3, 24 kHz mono.
pub const OUTPUT_FORMAT: &str = "audio-24khz-48kbitrate-mono-mp3";

/// Windows epoch offset (1601-01-01 vs 1970-01-01), in seconds.
const WIN_EPOCH_OFFSET_S: i64 = 11_644_473_600;

/// DRM token tick quantum: 5 minutes (reference client rounds DOWN).
const GEC_ROUND_S: i64 = 300;

// ---------------------------------------------------------------------------
// DRM token
// ---------------------------------------------------------------------------

/// The `Sec-MS-GEC` DRM token: SHA-256 over
/// `<ticks>+<TRUSTED_CLIENT_TOKEN>` as uppercase hex, where ticks is the
/// Windows file time of NOW rounded DOWN to the 5-minute boundary.
pub fn sec_ms_gec(unix_ms: u128) -> String {
    let secs = (unix_ms / 1000) as i64;
    let rounded = secs - secs.rem_euclid(GEC_ROUND_S);
    let ticks = (rounded + WIN_EPOCH_OFFSET_S) * 10_000_000;
    sha256_hex_upper(format!("{ticks}{TRUSTED_CLIENT_TOKEN}").as_bytes())
}

// ---------------------------------------------------------------------------
// Dial URL (GA1-gated)
// ---------------------------------------------------------------------------

/// Connection/request ids: 32 lowercase hex chars derived from a caller
/// seed (std has no RNG; callers mix time + counter; determinism is a
/// feature for tests and replay).
pub fn hex_id32(seed: &str) -> String {
    sha256_hex(seed.as_bytes())[..32].to_string()
}

/// Build the dial URL and authorize it (GA1). The returned plan is the
/// ONLY thing a transport may open.
pub fn dial_url(gen: &GeneratorSpec, gec: &str, connection_id: &str) -> Result<String, AdapterErr> {
    let url = format!(
        "wss://{WSS_HOST}/consumer/speech/synthesize/readaloud/edge/v1\
         ?TrustedClientToken={TRUSTED_CLIENT_TOKEN}\
         &Sec-MS-GEC={gec}\
         &Sec-MS-GEC-Version={SEC_MS_GEC_VERSION}\
         &ConnectionId={connection_id}"
    );
    let plan = authorize_dial(gen, &url)?;
    debug_assert_eq!(plan.host, WSS_HOST);
    Ok(url)
}

// ---------------------------------------------------------------------------
// Timestamp (ISO-8601 Z, millisecond precision)
// ---------------------------------------------------------------------------

/// Howard Hinnant's `civil_from_days` (public domain): days since
/// 1970-01-01 → (y, m, d).
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (y + i64::from(m <= 2), m, d)
}

/// `2026-08-27T18:35:29.123Z` from unix milliseconds.
pub fn iso_ts(unix_ms: u128) -> String {
    let ms_total = unix_ms % 1000;
    let secs_total = (unix_ms / 1000) as i64;
    let days = secs_total.div_euclid(86_400);
    let sod = secs_total.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    let (hh, mm, ss) = (sod / 3600, (sod % 3600) / 60, sod % 60);
    format!("{y:04}-{m:02}-{d:02}T{hh:02}:{mm:02}:{ss:02}.{ms_total:03}Z")
}

// ---------------------------------------------------------------------------
// Messages
// ---------------------------------------------------------------------------

/// SSML entity escaping — the text becomes DATA, never structure.
pub fn escape_ssml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            c => out.push(c),
        }
    }
    out
}

/// Message 1: the synthesis format + metadata options.
pub fn speech_config_msg(now_iso: &str) -> String {
    format!(
        "X-Timestamp:{now_iso}\r\n\
         Content-Type:application/json; charset=utf-8\r\n\
         Path:speech.config\r\n\r\n\
         {{\"context\":{{\"synthesis\":{{\"audio\":{{\"metadataoptions\":\
         {{\"sentenceBoundaryEnabled\":\"false\",\"wordBoundaryEnabled\":\"true\"}},\
         \"outputFormat\":\"{OUTPUT_FORMAT}\"}}}}}}}}"
    )
}

/// Voice prosody defaults (neutral, reference-client shape).
#[derive(Debug, Clone, PartialEq)]
pub struct Prosody {
    pub pitch: String,
    pub rate: String,
    pub volume: String,
}

impl Default for Prosody {
    fn default() -> Self {
        Prosody {
            pitch: "+0Hz".into(),
            rate: "+0%".into(),
            volume: "+0%".into(),
        }
    }
}

/// Message 2: the SSML synthesis request.
pub fn ssml_msg(
    request_id: &str,
    now_iso: &str,
    voice: &str,
    text: &str,
    prosody: &Prosody,
) -> String {
    format!(
        "X-RequestId:{request_id}\r\n\
         Content-Type:application/ssml+xml\r\n\
         X-Timestamp:{now_iso}\r\n\
         Path:ssml\r\n\r\n\
         <speak version='1.0' xmlns='http://www.w3.org/2001/10/synthesis' xml:lang='en-US'>\
         <voice name='{voice}'>\
         <prosody pitch='{pitch}' rate='{rate}' volume='{volume}'>{text}</prosody>\
         </voice></speak>",
        pitch = prosody.pitch,
        rate = prosody.rate,
        volume = prosody.volume,
        text = escape_ssml(text),
    )
}

// ---------------------------------------------------------------------------
// Frames
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq)]
pub enum WsFrame {
    Text(String),
    Binary(Vec<u8>),
}

/// The transport contract the schannel-backed WSS client (next slice)
/// implements. `set_read_timeout_ms` is part of the contract: the
/// transport MUST fail a read that exceeds it, so the R-D deadline is
/// enforced at the socket, not by a blocked caller.
pub trait WsStream {
    fn send_text(&mut self, s: &str) -> Result<(), String>;
    fn recv_frame(&mut self) -> Result<WsFrame, String>;
    fn set_read_timeout_ms(&mut self, ms: u32) -> Result<(), String>;
}

/// Parse a server binary frame: `u16be header_len | header | audio`.
/// Returns the audio slice only when the header says `Path:audio`.
pub fn parse_audio_frame(bytes: &[u8]) -> Result<Option<&[u8]>, AdapterErr> {
    if bytes.len() < 2 {
        return err("edge frame: shorter than the 2-byte header length");
    }
    let hlen = u16::from_be_bytes([bytes[0], bytes[1]]) as usize;
    let end = 2usize
        .checked_add(hlen)
        .ok_or_else(|| AdapterErr("edge frame: length overflow".into()))?;
    if bytes.len() < end {
        return err("edge frame: truncated header");
    }
    let header = &bytes[2..end];
    let header = std::str::from_utf8(header)
        .map_err(|_| AdapterErr("edge frame: non-UTF8 header".into()))?;
    // Full-line match: "Path:audio" is a PREFIX of "Path:audio.metadata"
    // — only the CRLF-terminated form is an audio frame.
    if header.contains("Path:audio\r\n") {
        Ok(Some(&bytes[end..]))
    } else {
        Ok(None)
    }
}

/// Extract `Path:<value>` from a server text frame's header block.
pub fn text_frame_path(msg: &str) -> Option<String> {
    let headers = msg.split("\r\n\r\n").next().unwrap_or("");
    for line in headers.split("\r\n") {
        if let Some(rest) = line.strip_prefix("Path:") {
            return Some(rest.trim().to_string());
        }
    }
    None
}

fn err<T>(msg: impl Into<String>) -> Result<T, AdapterErr> {
    Err(AdapterErr(msg.into()))
}

// ---------------------------------------------------------------------------
// Session
// ---------------------------------------------------------------------------

/// One synthesis session over an ESTABLISHED stream.
#[derive(Debug, Clone)]
pub struct SessionOpts {
    pub request_seed: String,
    /// R-D: single-attempt budget for the whole exchange.
    pub deadline_ms: u128,
    /// F-A4 telemetry cap.
    pub cap_ms: u32,
    pub prosody: Prosody,
}

/// Run one synthesis to completion. GA1 (dial authorization happens in
/// the caller when building the URL — this function receives the
/// GENERATOR only to stamp telemetry; the URL must already have passed
/// `dial_url`), text sanitization, R-D deadline, GA2 validation.
pub fn synthesize(
    stream: &mut dyn WsStream,
    gen: &GeneratorSpec,
    voice: &str,
    text: &str,
    opts: &SessionOpts,
) -> Result<RenderedAudio, AdapterErr> {
    let started = Instant::now();
    let s = sanitize_text(text)?;
    let rid = hex_id32(&opts.request_seed);
    let budget = opts.deadline_ms.max(1);
    stream
        .set_read_timeout_ms(u32::try_from(budget).unwrap_or(u32::MAX))
        .map_err(AdapterErr)?;
    stream
        .send_text(&speech_config_msg(&iso_ts(started.elapsed().as_millis())))
        .map_err(AdapterErr)?;
    stream
        .send_text(&ssml_msg(
            &rid,
            &iso_ts(started.elapsed().as_millis()),
            voice,
            &s.text,
            &opts.prosody,
        ))
        .map_err(AdapterErr)?;

    let mut audio: Vec<u8> = Vec::new();
    loop {
        let frame = stream
            .recv_frame()
            .map_err(|e| AdapterErr(format!("edge-tts: transport: {e}")))?;
        // R-D: the budget bounds the WHOLE exchange — a frame arriving
        // past the deadline (turn.end included) fails the attempt, never
        // completes it.
        if started.elapsed().as_millis() > budget {
            return err(format!(
                "edge-tts: R-D deadline {}ms exceeded with {} audio bytes",
                budget,
                audio.len()
            ));
        }
        match frame {
            WsFrame::Binary(b) => {
                if let Some(chunk) = parse_audio_frame(&b)? {
                    audio.extend_from_slice(chunk);
                    if audio.len() > crate::adapter::MAX_AUDIO_BYTES {
                        return err("edge-tts: audio stream exceeded the size cap mid-flight");
                    }
                }
            }
            WsFrame::Text(t) => {
                if text_frame_path(&t).as_deref() == Some("turn.end") {
                    break;
                }
            }
        }
    }
    validate_mp3(&audio)?;
    let elapsed_ms = started.elapsed().as_millis();
    Ok(RenderedAudio {
        bytes: audio,
        format: AudioFormat::Mp3,
        generator: gen.id.clone(),
        voice: voice.to_string(),
        elapsed_ms,
        cap_ms: opts.cap_ms,
        over_cap: elapsed_ms > u128::from(opts.cap_ms),
    }
    .over_cap_checked())
}

/// Exposed for tests + the transport slice: digest helper reuse.
pub fn token_digest(proof: &[u8]) -> [u8; 32] {
    sha256(proof)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::Lane;

    fn gen() -> GeneratorSpec {
        GeneratorSpec {
            id: "leonas".into(),
            lane: Lane::Network,
            startup_cap_ms: 100,
            render_cap_ms: 1500,
            declared_endpoints: vec![format!("wss://{WSS_HOST}")],
        }
    }

    #[test]
    fn drm_token_shape_and_rounding() {
        // A moment 29s INTO its 5-minute window.
        let ms: u128 = 1_769_608_529_000;
        let t = sec_ms_gec(ms);
        assert_eq!(t.len(), 64);
        assert!(
            t.chars()
                .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit()),
            "must be uppercase hex: {t}"
        );
        // Same token while still inside the window (+270s → 299s < 300)…
        assert_eq!(t, sec_ms_gec(ms + 270_000));
        // …and a different one past the boundary (+280s → 309s ≥ 300).
        assert_ne!(t, sec_ms_gec(ms + 280_000));
        // Boundary math: 18:35:29 → 18:35:00; verify via a hand-built ticks hash.
        let secs = (ms / 1000) as i64;
        let rounded = secs - secs.rem_euclid(300);
        assert_eq!(rounded % 300, 0);
        let ticks = (rounded + WIN_EPOCH_OFFSET_S) * 10_000_000;
        assert_eq!(
            t,
            sha256_hex_upper(format!("{ticks}{TRUSTED_CLIENT_TOKEN}").as_bytes())
        );
    }

    #[test]
    fn dial_url_passes_ga1_and_carries_params() {
        let url = dial_url(&gen(), "ABCD", "c1").unwrap();
        assert!(url.starts_with(
            "wss://speech.platform.bing.com/consumer/speech/synthesize/readaloud/edge/v1?"
        ));
        assert!(url.contains("TrustedClientToken=6A5AA1D4EAFF4E9FB37E23D68491D6F4"));
        assert!(url.contains("Sec-MS-GEC=ABCD"));
        assert!(url.contains(&format!("Sec-MS-GEC-Version={SEC_MS_GEC_VERSION}")));
        assert!(url.contains("ConnectionId=c1"));
        // GA1 fail-closed for a generator that declared nothing/other host.
        let mut bad = gen();
        bad.declared_endpoints = vec![];
        assert!(dial_url(&bad, "ABCD", "c1").is_err());
        bad.declared_endpoints = vec!["wss://elsewhere.example.com".into()];
        assert!(dial_url(&bad, "ABCD", "c1").is_err());
    }

    #[test]
    fn iso_timestamps_known_values() {
        assert_eq!(iso_ts(0), "1970-01-01T00:00:00.000Z");
        // 2026-08-27T18:35:29.123Z (hand-checked against UTC calendar).
        assert_eq!(iso_ts(1_787_855_729_123), "2026-08-27T18:35:29.123Z");
        // Leap-year day: 2024-02-29T12:00:00.000Z.
        assert_eq!(iso_ts(1_709_208_000_000), "2024-02-29T12:00:00.000Z");
        // Year boundary: 2025-12-31T23:59:59.999Z.
        assert_eq!(iso_ts(1_767_225_599_999), "2025-12-31T23:59:59.999Z");
    }

    #[test]
    fn ssml_messages_exact_shape() {
        let cfg = speech_config_msg("2026-08-27T18:35:29.000Z");
        assert!(cfg.starts_with("X-Timestamp:2026-08-27T18:35:29.000Z\r\nContent-Type:application/json; charset=utf-8\r\nPath:speech.config\r\n\r\n"));
        assert!(cfg.contains(&format!("\"outputFormat\":\"{OUTPUT_FORMAT}\"")));
        assert!(cfg.contains("\"wordBoundaryEnabled\":\"true\""));
        let m = ssml_msg(
            "rid123",
            "TS",
            "lt-LT-LeonasNeural",
            "Sveiki & labas",
            &Prosody::default(),
        );
        assert!(m.starts_with("X-RequestId:rid123\r\nContent-Type:application/ssml+xml\r\nX-Timestamp:TS\r\nPath:ssml\r\n\r\n"));
        assert!(m.contains("<voice name='lt-LT-LeonasNeural'>"));
        assert!(m.contains("pitch='+0Hz' rate='+0%' volume='+0%'"));
        assert!(m.contains("Sveiki &amp; labas"));
    }

    #[test]
    fn ssml_escapes_all_structure() {
        assert_eq!(
            escape_ssml("a<b>c&d\"e'f"),
            "a&lt;b&gt;c&amp;d&quot;e&apos;f"
        );
    }

    fn audio_frame(audio: &[u8]) -> WsFrame {
        let header = b"X-RequestId:ab\r\nPath:audio\r\nContent-Type:audio/mpeg\r\n";
        let mut b = vec![];
        b.extend_from_slice(&(header.len() as u16).to_be_bytes());
        b.extend_from_slice(header);
        b.extend_from_slice(audio);
        WsFrame::Binary(b)
    }

    #[test]
    fn audio_frame_parser_edges() {
        let f = audio_frame(b"\xFF\xFB\x90rest");
        match &f {
            WsFrame::Binary(b) => {
                assert_eq!(parse_audio_frame(b).unwrap().unwrap(), b"\xFF\xFB\x90rest");
            }
            _ => unreachable!(),
        }
        // Non-audio binary frames (e.g. keepalive metadata) → None.
        let header = b"Path:audio.metadata\r\n";
        let mut b = vec![];
        b.extend_from_slice(&(header.len() as u16).to_be_bytes());
        b.extend_from_slice(header);
        assert!(parse_audio_frame(&b).unwrap().is_none());
        // Truncated frames refused.
        assert!(parse_audio_frame(&[0x00]).is_err());
        assert!(parse_audio_frame(&[0x00, 0x05, b'P']).is_err());
    }

    #[test]
    fn text_frame_path_extraction() {
        let t = "X-RequestId:x\r\nPath:turn.end\r\n\r\n{}".to_string();
        assert_eq!(text_frame_path(&t).as_deref(), Some("turn.end"));
        let no = "Content-Type:application/json; charset=utf-8\r\n\r\n{}".to_string();
        assert_eq!(text_frame_path(&no), None);
    }

    /// Scripted mock transport: records sends, yields queued frames,
    /// optionally stalling each recv (the slow-lane shape).
    struct MockWs {
        sent: Vec<String>,
        frames: std::collections::VecDeque<WsFrame>,
        timeouts: Vec<u32>,
        stall_ms: u64,
    }

    impl WsStream for MockWs {
        fn send_text(&mut self, s: &str) -> Result<(), String> {
            self.sent.push(s.to_string());
            Ok(())
        }
        fn recv_frame(&mut self) -> Result<WsFrame, String> {
            if self.stall_ms > 0 {
                std::thread::sleep(std::time::Duration::from_millis(self.stall_ms));
            }
            self.frames.pop_front().ok_or_else(|| "closed".into())
        }
        fn set_read_timeout_ms(&mut self, ms: u32) -> Result<(), String> {
            self.timeouts.push(ms);
            Ok(())
        }
    }

    fn mp3() -> Vec<u8> {
        let mut v = vec![0xFF, 0xFB, 0x90, 0x00];
        v.extend_from_slice(&[0x51u8; 600]);
        v
    }

    #[test]
    fn full_session_happy_path() {
        let mp3 = mp3();
        let mut ws = MockWs {
            sent: vec![],
            frames: [
                WsFrame::Text("X-RequestId:x\r\nPath:turn.start\r\n\r\n{}".into()),
                audio_frame(&mp3[..300]),
                audio_frame(&mp3[300..]),
                WsFrame::Text("X-RequestId:x\r\nPath:turn.end\r\n\r\n{}".into()),
            ]
            .into_iter()
            .collect(),
            timeouts: vec![],
            stall_ms: 0,
        };
        let r = synthesize(
            &mut ws,
            &gen(),
            "lt-LT-LeonasNeural",
            "Sveiki, tai balsas.",
            &SessionOpts {
                request_seed: "seed-1".into(),
                deadline_ms: 2_500,
                cap_ms: 1_500,
                prosody: Prosody::default(),
            },
        )
        .unwrap();
        assert_eq!(r.format, AudioFormat::Mp3);
        assert_eq!(r.bytes, mp3);
        assert_eq!(r.voice, "lt-LT-LeonasNeural");
        assert_eq!(r.generator, "leonas");
        // Both handshake messages went out, in protocol order.
        assert_eq!(ws.sent.len(), 2);
        assert!(ws.sent[0].contains("Path:speech.config"));
        assert!(ws.sent[1].contains("Path:ssml"));
        assert!(ws.sent[1].contains("Sveiki, tai balsas."));
        // The transport got the R-D budget as its read timeout.
        assert_eq!(ws.timeouts, vec![2500]);
    }

    #[test]
    fn session_rejects_bad_text_before_any_send() {
        let mut ws = MockWs {
            sent: vec![],
            frames: std::collections::VecDeque::new(),
            timeouts: vec![],
            stall_ms: 0,
        };
        let e = synthesize(
            &mut ws,
            &gen(),
            "lt-LT-LeonasNeural",
            "<speak>injected</speak>",
            &SessionOpts {
                request_seed: "s".into(),
                deadline_ms: 2_500,
                cap_ms: 1_500,
                prosody: Prosody::default(),
            },
        )
        .unwrap_err();
        assert!(e.0.contains("markup"), "unexpected err: {e}");
        assert!(
            ws.sent.is_empty(),
            "no bytes may leave before text passes the guards"
        );
    }

    #[test]
    fn turn_end_without_audio_fails_ga2() {
        let mut ws = MockWs {
            sent: vec![],
            frames: [WsFrame::Text("Path:turn.end\r\n\r\n{}".into())]
                .into_iter()
                .collect(),
            timeouts: vec![],
            stall_ms: 0,
        };
        let e = synthesize(
            &mut ws,
            &gen(),
            "v",
            "tekstas",
            &SessionOpts {
                request_seed: "s".into(),
                deadline_ms: 2_500,
                cap_ms: 1_500,
                prosody: Prosody::default(),
            },
        )
        .unwrap_err();
        assert!(e.0.contains("GA2"), "unexpected err: {e}");
    }

    #[test]
    fn markup_body_discarded_not_returned() {
        // A lane that answers with an XML error body inside an audio frame:
        // GA2 must discard it.
        let mut ws = MockWs {
            sent: vec![],
            frames: [
                audio_frame(b"<?xml version='1.0'?><error/>"),
                WsFrame::Text("Path:turn.end\r\n\r\n{}".into()),
            ]
            .into_iter()
            .collect(),
            timeouts: vec![],
            stall_ms: 0,
        };
        let e = synthesize(
            &mut ws,
            &gen(),
            "v",
            "tekstas",
            &SessionOpts {
                request_seed: "s".into(),
                deadline_ms: 2_500,
                cap_ms: 1_500,
                prosody: Prosody::default(),
            },
        )
        .unwrap_err();
        assert!(
            e.0.contains("markup") || e.0.contains("GA2"),
            "unexpected err: {e}"
        );
    }

    #[test]
    fn r_d_deadline_fires_on_stalled_lane() {
        // A lane that answers, but SLOWLY: each frame takes 30ms against
        // a 35ms budget — after the second frame (60ms) the deadline
        // check must trip before the exchange completes (drill 6 shape).
        let mut ws = MockWs {
            sent: vec![],
            frames: [
                audio_frame(&mp3()),
                WsFrame::Text("Path:turn.end\r\n\r\n{}".into()),
            ]
            .into_iter()
            .collect(),
            timeouts: vec![],
            stall_ms: 30,
        };
        let e = synthesize(
            &mut ws,
            &gen(),
            "v",
            "tekstas",
            &SessionOpts {
                request_seed: "s".into(),
                deadline_ms: 35,
                cap_ms: 1_500,
                prosody: Prosody::default(),
            },
        )
        .unwrap_err();
        assert!(e.0.contains("R-D deadline"), "unexpected err: {e}");
    }

    #[test]
    fn hex_ids_are_32_chars_and_stable() {
        assert_eq!(hex_id32("a").len(), 32);
        assert_eq!(hex_id32("a"), hex_id32("a"));
        assert_ne!(hex_id32("a"), hex_id32("b"));
    }
}
