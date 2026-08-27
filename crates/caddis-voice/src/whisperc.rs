//! whisperc.rs — the whisper-server HTTP client (the Horn's engine lane).
//!
//! Speaks whisper.cpp server's one route — `POST /inference`, multipart in,
//! flat JSON `{"text": ...}` out — over a raw `TcpStream`, because the crate
//! is std-only by law and the dialect is four headers long.
//!
//! GA2 (quorum, applied to this lane): the upstream response is VALIDATED
//! before anything is returned — status 200, sane content-type, bounded body,
//! and `text` must be a JSON string. A lane that answers garbage gets a
//! typed error, never a passthrough. The operator's dictation must fail
//! loudly, not deliver nonsense.
//!
//! `one_line` is ported VERBATIM in intent from `stt_gpu.py`: whisper-server
//! joins segments with newlines and segment edges can fall inside a
//! Lithuanian word ("Casabl"/"ancos") — collapsing whitespace runs to ONE
//! space is what keeps delivered dictation a paragraph instead of confetti,
//! and replacing (never deleting) the separator is what keeps words unfused.

use crate::json::{self, Value};
use crate::multipart;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::time::Duration;

/// Connect timeout — a local engine that cannot connect in 2s is down.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// Inference budget — 30s dictation at RTF 0.35 is ~10s; 120 leaves room for
/// a cold model load while still being a bound (daemon-proven numbers).
pub const INFER_TIMEOUT: Duration = Duration::from_secs(120);
/// Response body ceiling for the engine lane (a transcript is KiB-class).
pub const RESPONSE_CAP: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WhisperErr {
    Connect(String),
    Write(String),
    Read(String),
    /// GA2: the lane answered, but not with a valid transcription document.
    BadResponse(String),
    /// The lane answered a non-200 (e.g. 500 mid-model-load).
    Status(u16),
}

impl std::fmt::Display for WhisperErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WhisperErr::Connect(c) => write!(f, "engine connect: {c}"),
            WhisperErr::Write(c) => write!(f, "engine write: {c}"),
            WhisperErr::Read(c) => write!(f, "engine read: {c}"),
            WhisperErr::BadResponse(c) => write!(f, "engine response invalid: {c}"),
            WhisperErr::Status(s) => write!(f, "engine status {s}"),
        }
    }
}

/// The normalised transcription result — the daemon contract shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Transcript {
    pub text: String,
    pub language: Option<String>,
    pub duration_s: f64,
    /// `segments` only when the caller asked for word_timestamps; otherwise
    /// a single synthetic segment when text exists (daemon normalise shape).
    pub segments: Vec<Segment>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub start_s: f64,
    pub end_s: f64,
    pub text: String,
}

/// Whitespace-run collapse + strip. See the module doc: the separator is
/// REPLACED with one space, never deleted.
pub fn one_line(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut in_ws = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            in_ws = true;
        } else {
            if in_ws && !out.is_empty() {
                out.push(' ');
            }
            in_ws = false;
            out.push(ch);
        }
    }
    out
}

/// POST one WAV (16-bit PCM bytes) to `host:port/inference` and return the
/// normalised transcript. `language` forwards whisper's `-l` hint per request
/// (the daemon contract: caller-chosen language beats the server default).
pub fn transcribe(
    host: &str,
    port: u16,
    wav_bytes: &[u8],
    language: Option<&str>,
    word_timestamps: bool,
    duration_s: f64,
) -> Result<Transcript, WhisperErr> {
    let mut fields: Vec<(&str, &str)> = vec![("response_format", "json"), ("temperature", "0.0")];
    if let Some(l) = language {
        fields.push(("language", l));
    }
    if word_timestamps {
        fields.push(("word_timestamps", "true"));
    }
    let (body, content_type) = multipart::build(wav_bytes, &fields)
        .map_err(|e| WhisperErr::Write(format!("boundary: {e}")))?;
    let raw = post_inference(host, port, &body, &content_type)?;
    normalise(&raw, duration_s, language)
}

/// One request/response cycle against the engine.
fn post_inference(
    host: &str,
    port: u16,
    body: &[u8],
    content_type: &str,
) -> Result<Value, WhisperErr> {
    let addr = format!("{host}:{port}");
    let mut stream = TcpStream::connect(&addr).map_err(|e| WhisperErr::Connect(e.to_string()))?;
    stream
        .set_read_timeout(Some(INFER_TIMEOUT))
        .map_err(|e| WhisperErr::Connect(e.to_string()))?;
    stream
        .set_write_timeout(Some(CONNECT_TIMEOUT))
        .map_err(|e| WhisperErr::Connect(e.to_string()))?;

    let head = format!(
        "POST /inference HTTP/1.1\r\nHost: {addr}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    stream
        .write_all(head.as_bytes())
        .and_then(|_| stream.write_all(body))
        .and_then(|_| stream.flush())
        .map_err(|e| WhisperErr::Write(e.to_string()))?;

    // Read to EOF (Connection: close) under the response cap.
    let mut raw = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                raw.extend_from_slice(&chunk[..n]);
                if raw.len() > RESPONSE_CAP {
                    return Err(WhisperErr::BadResponse("response over cap".into()));
                }
            }
            Err(e) => return Err(WhisperErr::Read(e.to_string())),
        }
    }

    // Split head from body.
    let split = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or_else(|| WhisperErr::BadResponse("no header/body split".into()))?;
    let head_text = String::from_utf8_lossy(&raw[..split]).to_string();
    let mut lines = head_text.split("\r\n");
    let status_line = lines
        .next()
        .ok_or_else(|| WhisperErr::BadResponse("empty status line".into()))?;
    let status: u16 = status_line
        .split(' ')
        .nth(1)
        .and_then(|s| s.parse().ok())
        .ok_or_else(|| {
            WhisperErr::BadResponse(format!("unparseable status line: {status_line:?}"))
        })?;
    if status != 200 {
        return Err(WhisperErr::Status(status));
    }
    // GA2: content-type must look like JSON; audio or html back is a defect.
    let ct_json = head_text.to_ascii_lowercase().contains("content-type:")
        && head_text
            .to_ascii_lowercase()
            .split("\r\n")
            .any(|l| l.starts_with("content-type:") && l.contains("json"));
    if !ct_json {
        return Err(WhisperErr::BadResponse("content-type is not json".into()));
    }

    let body_text = String::from_utf8_lossy(&raw[split + 4..]);
    json::parse(&body_text).map_err(|e| WhisperErr::BadResponse(format!("not JSON: {e:?}")))
}

/// whisper-server JSON -> the daemon contract shape (ported from
/// `stt_gpu.normalise`): missing pieces become empty, `text` is one-lined.
pub fn normalise(
    raw: &Value,
    duration_s: f64,
    language: Option<&str>,
) -> Result<Transcript, WhisperErr> {
    // GA2: `text` must exist and be a string. An engine answering without it
    // is a lane defect, and delivering "" would read as "silence" to the
    // operator — the worst failure shape (the drop-ledger law).
    let text_raw = raw
        .get("text")
        .and_then(|t| t.as_str())
        .ok_or_else(|| WhisperErr::BadResponse("'text' missing or not a string".into()))?;
    let text = one_line(text_raw);

    let mut segments = Vec::new();
    if let Some(arr) = raw.get("segments").and_then(|s| s.as_arr()) {
        for seg in arr {
            let offsets = seg.get("offsets");
            let edge = |direct: &str, millis: &str| -> f64 {
                if let Some(v) = seg.get(direct).and_then(|v| v.as_f64()) {
                    return v;
                }
                offsets
                    .and_then(|o| o.get(millis))
                    .and_then(|v| v.as_f64())
                    .map(|m| m / 1000.0)
                    .unwrap_or(0.0)
            };
            segments.push(Segment {
                start_s: edge("start", "from"),
                end_s: edge("end", "to"),
                text: seg
                    .get("text")
                    .and_then(|t| t.as_str())
                    .map(one_line)
                    .unwrap_or_default(),
            });
        }
    }
    if segments.is_empty() && !text.is_empty() {
        segments.push(Segment {
            start_s: 0.0,
            end_s: duration_s,
            text: text.clone(),
        });
    }

    Ok(Transcript {
        text,
        language: raw
            .get("language")
            .and_then(|l| l.as_str())
            .map(|s| s.to_string())
            .or_else(|| language.map(|s| s.to_string())),
        duration_s,
        segments,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Mock-engine helper: wait for the FIRST request bytes (client may not
    /// have written yet), then drain until it goes quiet. Unread request
    /// bytes turn our close into an RST that destroys the queued response;
    /// closing before ANY read races the client's write. Both are mock bugs,
    /// not engine bugs.
    fn drain_request(sock: &mut std::net::TcpStream) {
        use std::time::{Duration, Instant};
        sock.set_read_timeout(Some(Duration::from_millis(2000)))
            .unwrap();
        let mut chunk = [0u8; 65536];
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut saw_any = false;
        while Instant::now() < deadline {
            match sock.read(&mut chunk) {
                Ok(n) if n > 0 => saw_any = true,
                Ok(_) => break,                 // clean EOF
                Err(_) if !saw_any => continue, // still waiting for first bytes
                Err(_) => break,                // went quiet after data: request complete
            }
        }
    }
    #[test]
    fn one_line_collapses_runs_and_replaces_never_deletes() {
        assert_eq!(
            one_line("  labas   rytas\n\nketvirta\tvalanda  "),
            "labas rytas ketvirta valanda"
        );
        assert_eq!(one_line("Casabl\nancos"), "Casabl ancos"); // split word: space, not fusion
        assert_eq!(one_line(""), "");
        assert_eq!(one_line("   "), "");
    }

    #[test]
    fn normalise_enforces_text_string() {
        let good = json::parse(r#"{"text":" labas \n rytas "}"#).unwrap();
        let t = normalise(&good, 1.5, Some("lt")).unwrap();
        assert_eq!(t.text, "labas rytas");
        assert_eq!(t.language.as_deref(), Some("lt"));
        assert_eq!(t.duration_s, 1.5);
        assert_eq!(t.segments.len(), 1); // synthetic single segment

        let no_text = json::parse(r#"{"language":"lt"}"#).unwrap();
        assert!(normalise(&no_text, 1.0, None).is_err());
        let wrong_type = json::parse(r#"{"text":123}"#).unwrap();
        assert!(normalise(&wrong_type, 1.0, None).is_err());
    }

    #[test]
    fn normalise_maps_segments_and_offsets() {
        let raw = json::parse(
            r#"{"text":"a b","segments":[{"start":0.0,"end":1.0,"text":"a"},{"offsets":{"from":1000,"to":2000},"text":"b"}]}"#,
        )
        .unwrap();
        let t = normalise(&raw, 2.0, None).unwrap();
        assert_eq!(t.segments.len(), 2);
        assert_eq!((t.segments[1].start_s, t.segments[1].end_s), (1.0, 2.0));
    }

    /// Full-socket E2E against a mock engine that speaks the whisper dialect.
    /// Proves request build + response parse + GA2 rejections over real TCP.
    #[test]
    fn full_round_trip_against_mock_engine() {
        use std::io::Write as _;
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let engine = std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 4096];
            // Read until the head split, then Content-Length more (small bodies).
            loop {
                let n = sock.read(&mut chunk).unwrap();
                buf.extend_from_slice(&chunk[..n]);
                if let Some(split) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = String::from_utf8_lossy(&buf[..split]).to_string();
                    let cl: usize = head
                        .lines()
                        .find(|l| l.to_ascii_lowercase().starts_with("content-length:"))
                        .and_then(|l| l.trim().split(':').nth(1))
                        .and_then(|v| v.trim().parse().ok())
                        .unwrap_or(0);
                    if buf.len() >= split + 4 + cl {
                        break;
                    }
                }
            }
            let body = "{\"text\":\" labas \\n rytas \",\"language\":\"lt\"}".to_string();
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).unwrap();
        });

        let wav = vec![0u8; 64]; // content irrelevant to the mock
        let t = transcribe("127.0.0.1", port, &wav, Some("lt"), false, 0.5).unwrap();
        engine.join().unwrap();
        assert_eq!(t.text, "labas rytas");
        assert_eq!(t.language.as_deref(), Some("lt"));
    }

    #[test]
    fn ga2_rejects_non_json_content_type_and_bad_status() {
        use std::io::Write as _;
        for (status, ct, body) in [
            ("500 Internal Server Error", "application/json", "{}"),
            ("200 OK", "text/html", "<html>"),
        ] {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            let engine = std::thread::spawn(move || {
                let (mut sock, _) = listener.accept().unwrap();
                drain_request(&mut sock);
                let resp = format!("HTTP/1.1 {status}\r\nContent-Type: {ct}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len());
                sock.write_all(resp.as_bytes()).unwrap();
            });
            let err = transcribe("127.0.0.1", port, &[0u8; 8], None, false, 0.1).unwrap_err();
            engine.join().unwrap();
            assert!(
                matches!(err, WhisperErr::Status(500) | WhisperErr::BadResponse(_)),
                "expected typed GA2 failure, got {err:?}"
            );
        }
    }
}
