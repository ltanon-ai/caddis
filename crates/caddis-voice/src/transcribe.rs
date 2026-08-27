//! transcribe.rs — the POST /transcribe endpoint (the Horn's mouth).
//!
//! Contract-ported from `stt-daemon/stt_http.py` so parallel-run clients
//! (opener :8741 routes, mic.html) work against the organ unchanged:
//! multipart in — `file` (uploaded WAV) or `path` (file under an allowlisted
//! root) — JSON out `{transcript, text, engine, model, device, language,
//! duration, segments}`.
//!
//! Guard order is the daemon's, and each rejection keeps its daemon status:
//! 421 host / 401 token / 411+413 length / 400 multipart / 403 path / 400
//! audio / **429 busy** / 502 engine. The 429 exists because the horn is
//! single-flight BY DESIGN: one GPU, one model, one inference at a time — a
//! second concurrent request would queue behind a stranger's dictation and
//! blow every latency budget invisibly. Busy says so loudly, with
//! `Retry-After: 2` (the daemon's number).
//!
//! GA2 on the response side: only a VALIDATED transcript becomes 200; a lane
//! defect is 502, never a guessed empty string ("" reads as silence to a
//! listening operator — the drop-ledger law's worst shape).

use crate::guards::{self, GuardVerdict, TokenGuard};
use crate::json::{self, Value};
use crate::multipart;
use crate::whisperc;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// WAVs shorter than this are rejected: the daemon's quarter-second rule
/// (a "transcription" of nothing is indistinguishable from a broken mic).
pub const MIN_AUDIO_S: f64 = 0.25;
pub const ENGINE_NAME: &str = "whisper.cpp-vulkan";

/// One endpoint response, socket-free (pure, table-testable).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EndpointResponse {
    pub status: u16,
    pub body: String,
    /// Extra headers (e.g. Retry-After on 429).
    pub headers: Vec<(String, String)>,
}

impl EndpointResponse {
    fn json(status: u16, body: String) -> Self {
        EndpointResponse {
            status,
            body,
            headers: Vec::new(),
        }
    }
    fn err(status: u16, msg: &str) -> Self {
        Self::json(status, format!("{{\"error\":\"{msg}\"}}"))
    }
}

/// Everything the endpoint needs, cheap to share.
pub struct HornService {
    pub engine_host: String,
    pub engine_port: u16,
    pub model_name: String,
    /// Default language when the request carries none (settings.language).
    pub default_language: Option<String>,
    pub token: TokenGuard,
    pub listen_port: u16,
    /// Allowed roots for the `path` source field. Empty = path field OFF.
    pub path_allowlist: Vec<PathBuf>,
    /// Single-flight: one inference at a time on the one GPU.
    busy: AtomicBool,
    // Soak counters (F-A8 per-lane visibility; P4 reads these).
    pub requests: AtomicU64,
    pub ok: AtomicU64,
    pub busy_rejects: AtomicU64,
    pub engine_errors: AtomicU64,
}

impl HornService {
    pub fn new(
        engine_host: String,
        engine_port: u16,
        model_name: String,
        default_language: Option<String>,
        token: TokenGuard,
        listen_port: u16,
        path_allowlist: Vec<PathBuf>,
    ) -> Self {
        HornService {
            engine_host,
            engine_port,
            model_name,
            default_language,
            token,
            listen_port,
            path_allowlist,
            busy: AtomicBool::new(false),
            requests: AtomicU64::new(0),
            ok: AtomicU64::new(0),
            busy_rejects: AtomicU64::new(0),
            engine_errors: AtomicU64::new(0),
        }
    }

    /// Head-stage guard chain (before ANY body byte is read from a client
    /// that has not passed token). Order: host → token → length policy.
    pub fn guard_head(&self, headers: &[(String, String)]) -> Option<EndpointResponse> {
        if !guards::host_ok(headers, self.listen_port) {
            return Some(EndpointResponse::err(421, "bad host"));
        }
        if !self.token.check(headers) {
            return Some(EndpointResponse::err(401, "unauthorized"));
        }
        match guards::body_policy(headers) {
            GuardVerdict::Pass => None,
            // body_policy's error bodies are already {"error":"..."} documents
            // with the daemon's exact wording — reuse them verbatim.
            v => Some(EndpointResponse::json(v.status(), v.error_body())),
        }
    }

    /// Body stage: multipart → source → WAV sanity → single-flight → engine.
    pub fn handle_body(&self, headers: &[(String, String)], body: &[u8]) -> EndpointResponse {
        self.requests.fetch_add(1, Ordering::Relaxed);
        let ctype = guards::header(headers, "Content-Type").unwrap_or("");
        let boundary = match multipart::boundary_from_content_type(ctype) {
            Some(b) => b,
            None => return EndpointResponse::err(400, "expected multipart/form-data"),
        };
        let parts = match multipart::parse(body, &boundary) {
            Ok(p) => p,
            Err(e) => return EndpointResponse::err(400, &format!("multipart: {e}")),
        };

        // Source resolution: upload first, then allowlisted path.
        let (wav, source_desc) = match multipart::part(&parts, "file") {
            Some(f) if !f.data.is_empty() => (f.data.clone(), "upload".to_string()),
            Some(_) => return EndpointResponse::err(400, "empty file"),
            None => match multipart::field(&parts, "path") {
                Some(p) if !self.path_allowlist.is_empty() => {
                    let cand = Path::new(&p);
                    if !guards::path_under_allowed_root(cand, &self.path_allowlist) {
                        return EndpointResponse::err(403, "path not allowed");
                    }
                    match std::fs::read(cand) {
                        Ok(bytes) if !bytes.is_empty() => (bytes, format!("path:{p}")),
                        Ok(_) => return EndpointResponse::err(400, "empty file"),
                        Err(_) => return EndpointResponse::err(400, "path unreadable"),
                    }
                }
                Some(_) => return EndpointResponse::err(403, "path not allowed"),
                None => return EndpointResponse::err(400, "no 'file' or 'path' field"),
            },
        };

        // WAV sanity + duration gate.
        let wav_info = match wav_meta(&wav) {
            Some(w) => w,
            None => return EndpointResponse::err(400, "expected a RIFF/WAVE file"),
        };
        if wav_info.duration_s < MIN_AUDIO_S {
            return EndpointResponse::err(400, "audio too short");
        }

        // Single-flight.
        if self
            .busy
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Relaxed)
            .is_err()
        {
            self.busy_rejects.fetch_add(1, Ordering::Relaxed);
            return EndpointResponse {
                status: 429,
                body: "{\"error\":\"busy\"}".into(),
                headers: vec![("Retry-After".into(), "2".into())],
            };
        }
        let result = self.run_engine(&parts, &wav, wav_info.duration_s);
        self.busy.store(false, Ordering::Release);
        let _ = source_desc; // logged by the caller with the outcome

        match result {
            Ok(t) => {
                self.ok.fetch_add(1, Ordering::Relaxed);
                EndpointResponse::json(200, self.success_body(&t))
            }
            Err(e) => {
                self.engine_errors.fetch_add(1, Ordering::Relaxed);
                EndpointResponse::err(502, &format!("engine: {e}"))
            }
        }
    }

    fn run_engine(
        &self,
        parts: &[multipart::Part],
        wav: &[u8],
        duration_s: f64,
    ) -> Result<whisperc::Transcript, whisperc::WhisperErr> {
        let language = multipart::field(parts, "language")
            .filter(|l| !l.is_empty())
            .or_else(|| self.default_language.clone());
        let word_ts = multipart::field(parts, "word_timestamps")
            .map(|v| matches!(v.trim(), "1" | "true" | "yes" | "True"))
            .unwrap_or(false);
        whisperc::transcribe(
            &self.engine_host,
            self.engine_port,
            wav,
            language.as_deref(),
            word_ts,
            duration_s,
        )
    }

    /// The daemon's success shape, field for field.
    fn success_body(&self, t: &whisperc::Transcript) -> String {
        let segments = Value::Arr(
            t.segments
                .iter()
                .map(|s| {
                    Value::Obj(vec![
                        ("start".into(), Value::Num(s.start_s)),
                        ("end".into(), Value::Num(s.end_s)),
                        ("text".into(), Value::Str(s.text.clone())),
                    ])
                })
                .collect(),
        );
        let v = Value::Obj(vec![
            ("transcript".into(), Value::Str(t.text.clone())), // back-compat (hermes client)
            ("text".into(), Value::Str(t.text.clone())),
            ("engine".into(), Value::Str(ENGINE_NAME.into())),
            ("model".into(), Value::Str(self.model_name.clone())),
            ("device".into(), Value::Str("gpu".into())),
            (
                "language".into(),
                match &t.language {
                    Some(l) => Value::Str(l.clone()),
                    None => Value::Null,
                },
            ),
            ("duration".into(), Value::Num(t.duration_s)),
            ("segments".into(), segments),
        ]);
        json::to_string(&v)
    }
}

/// The WAV facts the endpoint gates on.
pub struct WavMeta {
    pub sample_rate: u32,
    pub channels: u16,
    pub duration_s: f64,
}

/// Minimal RIFF/WAVE header walk: `RIFF`+`WAVE`, `fmt ` (PCM/float), `data`.
/// Returns None for anything else — the organ's v1 dialect is WAV (what
/// mic.html and encode_wav produce); anything richer is a defect to report,
/// not a container format to adopt.
pub fn wav_meta(bytes: &[u8]) -> Option<WavMeta> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return None;
    }
    let mut pos = 12;
    let mut fmt: Option<(u32, u16, u16, u16)> = None; // (rate, channels, bits, format)
    let mut data_len: Option<usize> = None;
    while pos + 8 <= bytes.len() {
        let id = &bytes[pos..pos + 4];
        let size = u32::from_le_bytes(bytes[pos + 4..pos + 8].try_into().ok()?) as usize;
        let body = pos + 8;
        if id == b"fmt " && size >= 16 && body + 16 <= bytes.len() {
            let audio_format = u16::from_le_bytes(bytes[body..body + 2].try_into().ok()?);
            let channels = u16::from_le_bytes(bytes[body + 2..body + 4].try_into().ok()?);
            let rate = u32::from_le_bytes(bytes[body + 4..body + 8].try_into().ok()?);
            let bits = u16::from_le_bytes(bytes[body + 14..body + 16].try_into().ok()?);
            // 1 = PCM, 3 = IEEE float — the two shapes the pipeline produces.
            if audio_format != 1 && audio_format != 3 {
                return None;
            }
            fmt = Some((rate, channels, bits, audio_format));
        } else if id == b"data" {
            // Chunk size may exceed the buffer (streamed writers lie); clamp.
            data_len = Some(size.min(bytes.len().saturating_sub(body)));
        }
        // Chunks are word-aligned.
        pos = body + size.div_ceil(2) * 2;
    }
    let (rate, channels, bits, _format) = fmt?;
    let data_len = data_len?;
    if rate == 0 || channels == 0 {
        return None;
    }
    // Frame size from the DECLARED bit depth (16-bit PCM is the pipeline's
    // shape; float is 32). A non-integral byte width is not a WAV we accept.
    if bits == 0 || bits % 8 != 0 {
        return None;
    }
    let bytes_per_frame = channels as usize * (bits as usize / 8);
    Some(WavMeta {
        sample_rate: rate,
        channels,
        duration_s: data_len as f64 / (rate as f64 * bytes_per_frame as f64),
    })
}

/// Test support shared across the crate's test modules (httpd E2E needs the
/// same well-formed WAVs the endpoint tests use).
#[cfg(test)]
pub mod tests_support {
    /// 16-bit PCM mono WAV of `secs` at 16 kHz (silence).
    pub fn tiny_wav(secs: f64) -> Vec<u8> {
        let rate: u32 = 16_000;
        let frames = (secs * rate as f64) as usize;
        let data_len = frames * 2;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
        b.extend_from_slice(b"WAVE");
        b.extend_from_slice(b"fmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes()); // PCM
        b.extend_from_slice(&1u16.to_le_bytes()); // mono
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes()); // byte rate
        b.extend_from_slice(&2u16.to_le_bytes()); // block align
        b.extend_from_slice(&16u16.to_le_bytes()); // bits
        b.extend_from_slice(b"data");
        b.extend_from_slice(&(data_len as u32).to_le_bytes());
        b.extend(std::iter::repeat_n(0u8, data_len));
        b
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::transcribe::tests_support::tiny_wav;
    use std::io::{Read as _, Write as _};

    fn multipart_body(wav: &[u8], fields: &[(&str, &str)]) -> (Vec<u8>, String) {
        multipart::build(wav, fields).unwrap()
    }

    fn service(engine_port: u16, listen_port: u16, token_file: &Path) -> HornService {
        HornService::new(
            "127.0.0.1".into(),
            engine_port,
            "large-v3".into(),
            Some("lt".into()),
            TokenGuard::new(token_file),
            listen_port,
            vec![],
        )
    }

    fn setup(label: &str) -> (PathBuf, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("caddis-voice-tr-{}-{label}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tf = dir.join("token.txt");
        std::fs::write(&tf, "tok123").unwrap();
        (dir, tf)
    }

    fn mock_engine(text: &'static str) -> u16 {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut buf = Vec::new();
            let mut chunk = [0u8; 8192];
            loop {
                let n = sock.read(&mut chunk).unwrap_or(0);
                if n == 0 {
                    break;
                }
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
            let body = format!("{{\"text\":\"{text}\",\"language\":\"lt\"}}");
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            sock.write_all(resp.as_bytes()).unwrap();
        });
        port
    }

    #[test]
    fn wav_meta_parses_and_gates_duration() {
        let m = wav_meta(&tiny_wav(1.0)).unwrap();
        assert_eq!((m.sample_rate, m.channels), (16_000, 1));
        assert!((m.duration_s - 1.0).abs() < 0.01);
        assert!(wav_meta(b"not a wav").is_none());
        assert!(wav_meta(&tiny_wav(0.1)).unwrap().duration_s < MIN_AUDIO_S);
    }

    #[test]
    fn full_happy_path_upload() {
        let (dir, tf) = setup("full_happy_path_upload");
        let engine = mock_engine(" labas \\n rytas ");
        let svc = service(engine, 8785, &tf);
        let (body, ctype) = multipart_body(&tiny_wav(0.5), &[]);
        let headers = vec![
            ("Host".into(), "127.0.0.1:8785".into()),
            ("X-STT-Token".into(), "tok123".into()),
            ("Content-Type".into(), ctype),
            ("Content-Length".into(), body.len().to_string()),
        ];
        let resp = svc.handle_body(&headers, &body);
        assert_eq!(resp.status, 200, "body: {}", resp.body);
        let v = json::parse(&resp.body).unwrap();
        assert_eq!(v.get("text").and_then(|t| t.as_str()), Some("labas rytas"));
        assert_eq!(
            v.get("transcript").and_then(|t| t.as_str()),
            Some("labas rytas")
        );
        assert_eq!(v.get("engine").and_then(|t| t.as_str()), Some(ENGINE_NAME));
        assert_eq!(v.get("device").and_then(|t| t.as_str()), Some("gpu"));
        assert_eq!(svc.ok.load(Ordering::Relaxed), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn head_guards_reject_in_daemon_order() {
        let (dir, tf) = setup("head_guards_reject_in_daemon_order");
        let svc = service(9, 8785, &tf);
        let ok_host: Vec<(String, String)> = vec![("Host".into(), "127.0.0.1:8785".into())];
        let ok_full = vec![
            ("Host".into(), "127.0.0.1:8785".into()),
            ("X-STT-Token".into(), "tok123".into()),
            ("Content-Length".into(), "10".into()),
        ];
        assert!(svc.guard_head(&ok_full).is_none());

        let bad_host = vec![("Host".into(), "evil.example:8785".into())];
        assert_eq!(svc.guard_head(&bad_host).unwrap().status, 421);

        let no_token = vec![
            ("Host".into(), "127.0.0.1:8785".into()),
            ("Content-Length".into(), "10".into()),
        ];
        assert_eq!(svc.guard_head(&no_token).unwrap().status, 401);

        let chunked = vec![
            ("Host".into(), "127.0.0.1:8785".into()),
            ("X-STT-Token".into(), "tok123".into()),
            ("Transfer-Encoding".into(), "chunked".into()),
        ];
        assert_eq!(svc.guard_head(&chunked).unwrap().status, 411);
        // host passes, token missing -> 401 fires before any length policy
        assert_eq!(svc.guard_head(&ok_host).unwrap().status, 401);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn body_rejections_cover_daemon_statuses() {
        let (dir, tf) = setup("body_rejections_cover_daemon_statuses");
        let svc = service(9, 8785, &tf);
        let h = |ctype: &str, len: usize| {
            vec![
                ("Host".into(), "127.0.0.1:8785".into()),
                ("X-STT-Token".into(), "tok123".into()),
                ("Content-Type".into(), ctype.into()),
                ("Content-Length".into(), len.to_string()),
            ]
        };
        // Not multipart.
        let r = svc.handle_body(&h("application/json", 2), b"{}");
        assert_eq!(r.status, 400);
        // No file/path field.
        let (body, ctype) = multipart_body(&[], &[("language", "lt")]);
        let r = svc.handle_body(&h(&ctype, body.len()), &body);
        assert_eq!(r.status, 400);
        // Empty file part.
        let (body, ctype) = multipart::build(b"", &[]).unwrap();
        let r = svc.handle_body(&h(&ctype, body.len()), &body);
        assert_eq!(r.status, 400);
        // Audio too short.
        let (body, ctype) = multipart_body(&tiny_wav(0.1), &[]);
        let r = svc.handle_body(&h(&ctype, body.len()), &body);
        assert_eq!(r.status, 400);
        // Non-WAV bytes in the file part.
        let (body, ctype) = multipart_body(b"garbage-not-wav-but-long-enough-0123456789", &[]);
        let r = svc.handle_body(&h(&ctype, body.len()), &body);
        assert_eq!(r.status, 400);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn engine_failure_is_502_never_empty_200() {
        let (dir, tf) = setup("engine_failure_is_502_never_empty_200");
        // Engine port with nothing listening: connect error -> 502.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let dead_port = listener.local_addr().unwrap().port();
        drop(listener);
        let svc = service(dead_port, 8785, &tf);
        let (body, ctype) = multipart_body(&tiny_wav(0.5), &[]);
        let headers = vec![
            ("Host".into(), "127.0.0.1:8785".into()),
            ("X-STT-Token".into(), "tok123".into()),
            ("Content-Type".into(), ctype),
            ("Content-Length".into(), body.len().to_string()),
        ];
        let r = svc.handle_body(&headers, &body);
        assert_eq!(r.status, 502);
        assert_eq!(svc.engine_errors.load(Ordering::Relaxed), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn path_field_respects_allowlist() {
        let (dir, tf) = setup("path_field_respects_allowlist");
        let root = dir.join("wavs");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("a.wav"), tiny_wav(0.5)).unwrap();
        std::fs::write(dir.join("outside.wav"), tiny_wav(0.5)).unwrap();
        let engine = mock_engine("ok");
        let mut svc = service(engine, 8785, &tf);
        svc.path_allowlist = vec![root.clone()];

        // Allowed path -> 200 through the real engine mock.
        let (body, ctype) =
            multipart::build_fields(&[("path", root.join("a.wav").to_str().unwrap())]).unwrap();
        let headers = vec![
            ("Host".into(), "127.0.0.1:8785".into()),
            ("X-STT-Token".into(), "tok123".into()),
            ("Content-Type".into(), ctype),
            ("Content-Length".into(), body.len().to_string()),
        ];
        let r = svc.handle_body(&headers, &body);
        assert_eq!(r.status, 200, "body: {}", r.body);

        // Outside the allowlist -> 403.
        let (body, ctype) =
            multipart::build_fields(&[("path", dir.join("outside.wav").to_str().unwrap())])
                .unwrap();
        let headers = vec![
            ("Host".into(), "127.0.0.1:8785".into()),
            ("X-STT-Token".into(), "tok123".into()),
            ("Content-Type".into(), ctype),
            ("Content-Length".into(), body.len().to_string()),
        ];
        let r = svc.handle_body(&headers, &body);
        assert_eq!(r.status, 403);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn single_flight_returns_429_with_retry_after() {
        let (dir, tf) = setup("single_flight_returns_429_with_retry_after");
        // Engine that holds the connection open until told.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let release = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let rel2 = release.clone();
        std::thread::spawn(move || {
            let (mut sock, _) = listener.accept().unwrap();
            let mut chunk = [0u8; 65536];
            // Short read timeout so the release flag is re-checked; an
            // untimed read would sleep past the release forever.
            let _ = sock.set_read_timeout(Some(std::time::Duration::from_millis(50)));
            while !rel2.load(Ordering::Relaxed) {
                let _ = sock.read(&mut chunk); // drain request as it arrives
            }
            let body = "{\"text\":\"done\"}";
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            let _ = sock.write_all(resp.as_bytes());
        });
        let svc = std::sync::Arc::new(service(port, 8785, &tf));
        let (body, ctype) = multipart_body(&tiny_wav(0.5), &[]);
        let headers: Vec<(String, String)> = vec![
            ("Host".into(), "127.0.0.1:8785".into()),
            ("X-STT-Token".into(), "tok123".into()),
            ("Content-Type".into(), ctype),
            ("Content-Length".into(), body.len().to_string()),
        ];
        let svc2 = svc.clone();
        let headers2 = headers.clone();
        let body2 = body.clone();
        let first = std::thread::spawn(move || svc2.handle_body(&headers2, &body2));
        std::thread::sleep(std::time::Duration::from_millis(150)); // let it engage the engine
        let r = svc.handle_body(&headers, &body);
        assert_eq!(r.status, 429);
        assert!(r
            .headers
            .iter()
            .any(|(k, v)| k == "Retry-After" && v == "2"));
        assert_eq!(svc.busy_rejects.load(Ordering::Relaxed), 1);
        release.store(true, Ordering::Relaxed);
        assert_eq!(first.join().unwrap().status, 200);
        std::fs::remove_dir_all(&dir).ok();
    }
}
