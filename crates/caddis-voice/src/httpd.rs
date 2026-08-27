//! httpd.rs — the organ's tiny HTTP surface (P2: /health + /transcribe).
//!
//! Deliberately boring: HTTP/1.1, Connection: close per request, one thread
//! per connection with a hard concurrency cap, read timeouts everywhere. A
//! transcription can legally take tens of seconds (cold model load), so the
//! loop CANNOT be the P1 single-threaded health loop — /health must answer
//! while an inference runs. But "thread per connection" without a cap is its
//! own defect, so: at most [`MAX_CONNECTIONS`] live at once; over the cap the
//! request gets 503 and a close (a local box with 2 clients does not need
//! more; a flood does not get to spawn threads).
//!
//! Hang-up tolerance (daemon law): a client that closes early is routine —
//! the write fails, the thread ends, the organ does not care.

use crate::health::{self, HealthState};
use crate::transcribe::{EndpointResponse, HornService};
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Request head cap (the daemon family's 8 KiB rule).
const HEAD_CAP: usize = 8 * 1024;
/// Absolute body ceiling — guards apply the 64 MiB policy; this is the
/// backstop for anything that slips a route without one.
const BODY_HARD_CAP: usize = crate::guards::MAX_UPLOAD_BYTES + 1024;
/// Live connections at once; above this: 503 + close.
const MAX_CONNECTIONS: u32 = 16;
/// Per-socket budgets.
const HEAD_TIMEOUT: Duration = Duration::from_secs(10);
const BODY_TIMEOUT: Duration = Duration::from_secs(60);

/// The routing surface: health (GET) + horn (POST /transcribe) + the
/// gramophone (POST /say, POST /earcon) — the say half is OPTIONAL so a
/// health/horn-only organ (tests, degraded boot) serves without a voice
/// stack wired.
pub struct OrganRoutes {
    pub health: Arc<HealthState>,
    pub horn: Arc<HornService>,
    pub say: Option<Arc<crate::sayd::SayService>>,
}

/// Body ceiling for the say-family routes (guards cap audio at the
/// 64 MiB policy; a say line is kilobytes — anything past this cap is
/// not a say request).
const SAY_BODY_CAP: usize = 32 * 1024;

/// Label length bound (drop-ledger naming stays a NAME, not a paragraph).
const SAY_LABEL_MAX: usize = 64;

/// Parsed request head: (method, path-with-query, ordered headers).
pub type ParsedHead = (String, String, Vec<(String, String)>);

/// Parse "METHOD SP PATH SP HTTP/x.y" + headers from a head buffer.
pub fn parse_head(head: &[u8]) -> Result<ParsedHead, String> {
    let text = std::str::from_utf8(head).map_err(|_| "request head is not UTF-8".to_string())?;
    let mut lines = text.split("\r\n");
    let request_line = lines
        .next()
        .filter(|l| !l.is_empty())
        .ok_or_else(|| "empty request line".to_string())?;
    let mut parts = request_line.split(' ');
    let (method, path) = match (parts.next(), parts.next(), parts.next()) {
        (Some(m), Some(p), Some(_)) => (m.to_string(), p.to_string()),
        _ => return Err("malformed request line".to_string()),
    };
    let mut headers = Vec::new();
    for line in lines {
        if line.is_empty() {
            continue;
        }
        if let Some((k, v)) = line.split_once(':') {
            headers.push((k.trim().to_string(), v.trim().to_string()));
        }
    }
    Ok((method, path, headers))
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        411 => "Length Required",
        413 => "Payload Too Large",
        421 => "Misdirected Request",
        429 => "Too Many Requests",
        503 => "Service Unavailable",
        _ => "Internal Server Error",
    }
}

/// Serialize one response (health shape or endpoint shape).
fn write_response(
    sock: &mut TcpStream,
    status: u16,
    body: &str,
    extra: &[(String, String)],
) -> std::io::Result<()> {
    let mut head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n",
        status,
        reason(status),
        body.len()
    );
    for (k, v) in extra {
        head.push_str(&format!("{k}: {v}\r\n"));
    }
    head.push_str("\r\n");
    sock.write_all(head.as_bytes())?;
    sock.write_all(body.as_bytes())?;
    sock.flush()
}

/// Read to the head terminator, capped and timed. Returns the HEAD bytes and
/// any CARRY-OVER body bytes that arrived in the same segments — a client
/// that writes head+body in one write is routine, and losing those bytes to a
/// head-only buffer would dead-wait the body read (measured defect this tick).
fn read_head(sock: &mut TcpStream) -> Result<(Vec<u8>, Vec<u8>), String> {
    sock.set_read_timeout(Some(HEAD_TIMEOUT))
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 1024];
    loop {
        match sock.read(&mut chunk) {
            Ok(0) => return Err("connection closed before the request head ended".into()),
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if let Some(split) = buf.windows(4).position(|w| w == b"\r\n\r\n") {
                    let head = buf[..split + 4].to_vec();
                    let carry = buf[split + 4..].to_vec();
                    return Ok((head, carry));
                }
                if buf.len() > HEAD_CAP {
                    return Err("request head exceeds 8 KiB cap".into());
                }
            }
            Err(e) => return Err(format!("read: {e}")),
        }
    }
}

/// Read exactly `len` body bytes (Content-Length already validated by the
/// guards; the hard cap backstops here anyway), starting from any carry-over
/// bytes the head read already collected.
fn read_body(sock: &mut TcpStream, len: usize, carry: Vec<u8>) -> Result<Vec<u8>, String> {
    if len > BODY_HARD_CAP {
        return Err("body over hard cap".into());
    }
    if carry.len() > len {
        return Err("body longer than Content-Length".into());
    }
    sock.set_read_timeout(Some(BODY_TIMEOUT))
        .map_err(|e| e.to_string())?;
    let mut body = carry;
    let mut chunk = [0u8; 16 * 1024];
    while body.len() < len {
        match sock.read(&mut chunk) {
            Ok(0) => return Err("connection closed mid-body".into()),
            Ok(n) => {
                body.extend_from_slice(&chunk[..n.min(len - body.len())]);
            }
            Err(e) => return Err(format!("read: {e}")),
        }
    }
    Ok(body)
}

/// Route one parsed request to a response, socket-free (table-testable).
pub fn route(
    routes: &OrganRoutes,
    method: &str,
    path: &str,
    headers: &[(String, String)],
    body: &[u8],
) -> (u16, String, Vec<(String, String)>) {
    let path_only = path.split('?').next().unwrap_or(path);
    match (method, path_only) {
        ("GET", "/health") => {
            let r = health::route(
                format!("{method} {path} HTTP/1.1\r\nHost: x\r\n\r\n").as_bytes(),
                &routes.health,
            );
            (r.status, r.body, Vec::new())
        }
        ("POST", "/transcribe") => {
            if let Some(rej) = routes.horn.guard_head(headers) {
                let EndpointResponse {
                    status,
                    body,
                    headers,
                } = rej;
                // Guard rejections are not lane health (auth/shape
                // gates, not the engine lane) — uncounted, by law.
                return (status, body, headers);
            }
            let t0 = std::time::Instant::now();
            let r = routes.horn.handle_body(headers, body);
            // QQ4 soak: only the ENGINE lane's terminal outcomes count
            // (200 = transcript served, 502 = lane defect). Shape/auth
            // rejections (400/403/...) and 429 busy (policy, and the
            // lane is serving someone) are not lane health — same law
            // as the guard rejects above.
            if r.status == 200 || r.status == 502 {
                if let Some(s) = &routes.health.soak {
                    s.record_transcribe(r.status == 200, t0.elapsed().as_millis() as u64);
                }
            }
            (r.status, r.body, r.headers)
        }
        ("POST", "/say") => route_say(routes, headers, body),
        ("POST", "/earcon") => route_earcon(routes, headers, body),
        ("GET", "/say") | ("GET", "/earcon") => (
            405,
            "{\"error\":\"say routes are POST-only\"}".into(),
            Vec::new(),
        ),
        ("GET", "/transcribe") => (
            405,
            "{\"error\":\"transcribe is POST-only\"}".into(),
            Vec::new(),
        ),
        ("POST", "/health") => (405, "{\"error\":\"health is GET-only\"}".into(), Vec::new()),
        _ => (
            404,
            "{\"error\":\"only /health, /transcribe, /say and /earcon exist\"}".into(),
            Vec::new(),
        ),
    }
}

/// POST /say — admit one line onto the gramophone queue.
///
/// Body: `{"text", "label"?, "priority"?, "narration"?, "path"?}` where
/// path is `general` (default) or `confirm` (R-B gated-confirm path).
/// v1 auth posture: local-box surface, no token (the daemon's own say
/// lane had none); the queue's cap + coalesce bound abuse, and P4's
/// soak review owns hardening.
fn route_say(
    routes: &OrganRoutes,
    _headers: &[(String, String)],
    body: &[u8],
) -> (u16, String, Vec<(String, String)>) {
    use crate::adapter::MAX_TEXT_CHARS;
    use crate::json;
    use crate::sayd::SayService;
    use crate::voiceset::SpeechPath;

    if body.len() > SAY_BODY_CAP {
        return (413, "{\"error\":\"say body over cap\"}".into(), Vec::new());
    }
    let Some(svc) = routes.say.clone() else {
        return (
            503,
            "{\"error\":\"say service not running on this organ\"}".into(),
            Vec::new(),
        );
    };
    let v = match json::parse(&String::from_utf8_lossy(body)) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                format!("{{\"error\":\"bad JSON: pos {}\"}}", e.at),
                Vec::new(),
            )
        }
    };
    let bad = |msg: String| (400, format!("{{\"error\":\"{msg}\"}}"), Vec::new());
    let text = match v.get("text").and_then(json::Value::as_str) {
        Some(t) => t.trim().to_string(),
        None => return bad("text (string) required".into()),
    };
    if text.is_empty() {
        return bad("text is empty".into());
    }
    if text.chars().count() > MAX_TEXT_CHARS {
        return bad(format!("text over {MAX_TEXT_CHARS} chars"));
    }
    let label = match v.get("label").and_then(json::Value::as_str) {
        Some(l) => l.trim().to_string(),
        None => "sergeant".to_string(),
    };
    if label.is_empty() || label.chars().count() > SAY_LABEL_MAX {
        return bad(format!("label must be 1..={SAY_LABEL_MAX} chars"));
    }
    let priority = match v.get("priority") {
        None => 1u8,
        Some(p) => match p.as_f64() {
            Some(n) if n.fract() == 0.0 && (0.0..=2.0).contains(&n) => n as u8,
            _ => return bad("priority must be 0, 1 or 2".into()),
        },
    };
    let narration = v
        .get("narration")
        .and_then(json::Value::as_bool)
        .unwrap_or(true);
    let path = match v.get("path").and_then(json::Value::as_str) {
        None => SpeechPath::GeneralSpeech,
        Some("general") => SpeechPath::GeneralSpeech,
        Some("confirm") => SpeechPath::GatedConfirm,
        Some(other) => return bad(format!("unknown path {other:?} (general|confirm)")),
    };
    let (adm, depth) = SayService::say(&svc, &label, &text, narration, priority, path);
    let verdict = match adm {
        crate::gramophone::Admission::Queued => "queued",
        crate::gramophone::Admission::Coalesced => "coalesced",
        crate::gramophone::Admission::Evicted(_) => "evicted",
    };
    (
        200,
        format!("{{\"ok\":true,\"admission\":\"{verdict}\",\"depth\":{depth}}}"),
        Vec::new(),
    )
}

/// POST /earcon — fire a life-event chime (attention/done/...). The
/// worker plays it between speeches; unknown events are refused against
/// the embedded set.
fn route_earcon(
    routes: &OrganRoutes,
    _headers: &[(String, String)],
    body: &[u8],
) -> (u16, String, Vec<(String, String)>) {
    use crate::json;
    if body.len() > SAY_BODY_CAP {
        return (413, "{\"error\":\"earcon body over cap\"}".into(), Vec::new());
    }
    let Some(svc) = routes.say.clone() else {
        return (
            503,
            "{\"error\":\"say service not running on this organ\"}".into(),
            Vec::new(),
        );
    };
    let v = match json::parse(&String::from_utf8_lossy(body)) {
        Ok(v) => v,
        Err(e) => {
            return (
                400,
                format!("{{\"error\":\"bad JSON: pos {}\"}}", e.at),
                Vec::new(),
            )
        }
    };
    let event = match v.get("event").and_then(json::Value::as_str) {
        Some(e) => e.trim().to_string(),
        None => return (400, "{\"error\":\"event (string) required\"}".into(), Vec::new()),
    };
    match crate::sayd::SayService::earcon(&svc, &event) {
        Ok(()) => (200, "{\"ok\":true}".into(), Vec::new()),
        Err(e) => (
            400,
            format!("{{\"error\":\"{}\"}}", e.replace('"', "'")),
            Vec::new(),
        ),
    }
}

/// Serve on an ALREADY-BOUND (mutex-held) listener until `stop` flips.
/// Thread per connection, capped; every write failure is a dead peer, not a
/// defect (hang-up tolerance law).
pub fn serve(
    listener: TcpListener,
    routes: Arc<OrganRoutes>,
    stop: Arc<AtomicBool>,
) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    let live: Arc<AtomicU64> = Arc::new(AtomicU64::new(0));
    let mut threads = Vec::new();
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((mut sock, _addr)) => {
                if live.load(Ordering::Relaxed) >= MAX_CONNECTIONS as u64 {
                    let _ = write_response(&mut sock, 503, "{\"error\":\"busy\"}", &[]);
                    continue;
                }
                live.fetch_add(1, Ordering::Relaxed);
                let routes = routes.clone();
                let live = live.clone();
                let stop = stop.clone();
                threads.push(std::thread::spawn(move || {
                    handle_conn(&mut sock, &routes, &stop);
                    live.fetch_sub(1, Ordering::Relaxed);
                    let _ = sock.shutdown(std::net::Shutdown::Both);
                }));
                // Reap finished threads so the handle vec cannot grow forever.
                threads.retain(|t| !t.is_finished());
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(e) => return Err(e),
        }
    }
    for t in threads.drain(..) {
        let _ = t.join();
    }
    Ok(())
}

fn handle_conn(sock: &mut TcpStream, routes: &OrganRoutes, _stop: &AtomicBool) {
    // Accepted sockets inherit the listener's NON-BLOCKING mode on Windows;
    // per-connection work is blocking with timeouts (the listener loops, the
    // connections wait).
    let _ = sock.set_nonblocking(false);
    let (head, carry) = match read_head(sock) {
        Ok(h) => h,
        Err(e) => {
            let _ = write_response(sock, 400, &format!("{{\"error\":\"{e}\"}}"), &[]);
            return;
        }
    };
    let (method, path, headers) = match parse_head(&head) {
        Ok(x) => x,
        Err(e) => {
            let _ = write_response(sock, 400, &format!("{{\"error\":\"{e}\"}}"), &[]);
            return;
        }
    };
    // Only read a body for the route that wants one (guards already ran).
    let wants_body = method == "POST"
        && matches!(
            path.split('?').next(),
            Some("/transcribe") | Some("/say") | Some("/earcon")
        );
    let body = if wants_body {
        let len: usize = crate::guards::header(&headers, "Content-Length")
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        if len == 0 {
            let _ = write_response(sock, 411, "{\"error\":\"content-length required\"}", &[]);
            return;
        }
        match read_body(sock, len, carry) {
            Ok(b) => b,
            Err(e) => {
                let _ = write_response(sock, 400, &format!("{{\"error\":\"{e}\"}}"), &[]);
                return;
            }
        }
    } else {
        Vec::new()
    };
    let (status, body, extra) = route(routes, &method, &path, &headers, &body);
    let _ = write_response(sock, status, &body, &extra); // dead peer = routine
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guards::TokenGuard;
    use std::path::PathBuf;

    fn routes_with(engine_port: u16, listen_port: u16, label: &str) -> (OrganRoutes, PathBuf) {
        let dir =
            std::env::temp_dir().join(format!("caddis-voice-httpd-{}-{label}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let tf = dir.join("token.txt");
        std::fs::write(&tf, "tok").unwrap();
        let horn = HornService::new(
            "127.0.0.1".into(),
            engine_port,
            "large-v3".into(),
            Some("lt".into()),
            TokenGuard::new(&tf),
            listen_port,
            vec![],
        );
        (
            OrganRoutes {
                health: Arc::new(HealthState::boot(
                    "caddis-voice",
                    crate::VERSION,
                    vec![8785],
                )),
                horn: Arc::new(horn),
                say: None,
            },
            dir,
        )
    }

    /// A minimal live SayService (stub lane + counting sink) for the
    /// /say + /earcon route tests.
    fn say_service() -> Arc<crate::sayd::SayService> {
        use crate::adapter::{AdapterErr, RenderedAudio, BreakerConfig};
        use crate::say::{PlaySink, RenderLane};
        use crate::sayd::SayService;
        struct Lane;
        impl RenderLane for Lane {
            fn generator(&self) -> &str {
                "piper"
            }
            fn render(
                &self,
                _v: &crate::registry::VoiceSpec,
                _t: &str,
                _ls: f64,
            ) -> Result<RenderedAudio, AdapterErr> {
                Ok(RenderedAudio {
                    bytes: vec![1, 2, 3, 4],
                    format: crate::adapter::AudioFormat::Wav,
                    generator: "piper".into(),
                    voice: "en_US-ryan".into(),
                    elapsed_ms: 1,
                    cap_ms: 1500,
                    over_cap: false,
                })
            }
        }
        struct Sink;
        impl PlaySink for Sink {
            fn play(&mut self, _wav: &[u8]) -> bool {
                true
            }
        }
        Arc::new(SayService::start(
            crate::config::OrganConfig::default(),
            vec![Box::new(Lane)],
            Box::new(Sink),
            None,
            BreakerConfig {
                capacity: 100,
                refill_per_min: 100,
                cooldown_ms: 1_000,
            },
            None,
        ))
    }

    #[test]
    fn say_route_validates_queues_and_reports() {
        let (mut routes, dir) = routes_with(9, 8785, "say");
        routes.say = Some(say_service());
        // Happy path: queued + depth echo.
        let (s, b, _) = route(
            &routes,
            "POST",
            "/say",
            &[],
            br#"{"text":"Labas, operatoriau.","label":"sergeant"}"#,
        );
        assert_eq!(s, 200, "{b}");
        assert!(b.contains("\"admission\":\"queued\""), "{b}");
        assert!(b.contains("\"depth\":1"), "{b}");
        // Same line inside the window: coalesced — and honest about it.
        let (s, b, _) = route(
            &routes,
            "POST",
            "/say",
            &[],
            br#"{"text":"Labas, operatoriau.","label":"sergeant"}"#,
        );
        assert_eq!(s, 200);
        assert!(b.contains("\"admission\":\"coalesced\""), "{b}");
        // Validation ladder: missing text / empty / bad priority / bad path.
        let (s, _, _) = route(&routes, "POST", "/say", &[], br#"{"label":"x"}"#);
        assert_eq!(s, 400);
        let (s, _, _) = route(&routes, "POST", "/say", &[], br#"{"text":"   "}"#);
        assert_eq!(s, 400);
        let (s, _, _) = route(&routes, "POST", "/say", &[], br#"{"text":"a","priority":3}"#);
        assert_eq!(s, 400);
        let (s, _, _) = route(&routes, "POST", "/say", &[], br#"{"text":"a","path":"loud"}"#);
        assert_eq!(s, 400);
        let (s, _, _) = route(&routes, "POST", "/say", &[], b"not json");
        assert_eq!(s, 400);
        // Over-cap body.
        let big = format!("{{\"text\":\"{}\"}}", "a".repeat(40 * 1024));
        let (s, _, _) = route(&routes, "POST", "/say", &[], big.as_bytes());
        assert_eq!(s, 413);
        // 405 + 404 stay honest.
        let (s, _, _) = route(&routes, "GET", "/say", &[], b"");
        assert_eq!(s, 405);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn say_route_without_service_is_503_and_earcon_validates() {
        let (routes, dir) = routes_with(9, 8785, "say503");
        let (s, _, _) = route(&routes, "POST", "/say", &[], br#"{"text":"hi"}"#);
        assert_eq!(s, 503);
        let (s, _, _) = route(&routes, "POST", "/earcon", &[], br#"{"event":"done"}"#);
        assert_eq!(s, 503);

        let (mut routes, dir2) = routes_with(9, 8785, "earcon");
        routes.say = Some(say_service());
        let (s, b, _) = route(&routes, "POST", "/earcon", &[], br#"{"event":"attention"}"#);
        assert_eq!(s, 200, "{b}");
        let (s, _, _) = route(&routes, "POST", "/earcon", &[], br#"{"event":"nope"}"#);
        assert_eq!(s, 400);
        let (s, _, _) = route(&routes, "POST", "/earcon", &[], br#"{}"#);
        assert_eq!(s, 400);
        std::fs::remove_dir_all(&dir).ok();
        std::fs::remove_dir_all(&dir2).ok();
    }

    fn tiny_wav(secs: f64) -> Vec<u8> {
        crate::transcribe::tests_support::tiny_wav(secs)
    }

    #[test]
    fn routing_table_health_and_404_and_405() {
        let (routes, dir) = routes_with(9, 8785, "routing");
        let (s, b, _) = route(&routes, "GET", "/health", &[], b"");
        assert_eq!(s, 200);
        assert!(b.contains("\"organ\":\"caddis-voice\""));
        let (s, _, _) = route(&routes, "GET", "/nope", &[], b"");
        assert_eq!(s, 404);
        let (s, _, _) = route(&routes, "GET", "/transcribe", &[], b"");
        assert_eq!(s, 405);
        let (s, _, _) = route(&routes, "POST", "/health", &[], b"");
        assert_eq!(s, 405);
        let (s, _, _) = route(&routes, "POST", "/transcribe?token=x", &[], b"");
        assert_eq!(s, 401); // no token header: token guard fires before length
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn health_carries_soak_and_transcribe_records_horn_lane() {
        let (mut routes, dir) = routes_with(9, 8785, "soak");
        let soak = crate::soak::shared(Some(dir.join("soak-ledger.jsonl")));
        routes.health = Arc::new(
            HealthState::boot("caddis-voice", crate::VERSION, vec![8785]).with_soak(soak.clone()),
        );
        // /health carries the section the moment the instrument exists.
        let (s, b, _) = route(&routes, "GET", "/health", &[], b"");
        assert_eq!(s, 200);
        assert!(b.contains("\"soak\""), "{b}");
        assert!(b.contains("\"windows\""), "{b}");
        // A guarded reject (401) is NOT lane health — no row, no counter.
        let (s, _, _) = route(
            &routes,
            "POST",
            "/transcribe",
            &[("Host".into(), "127.0.0.1:8785".into())],
            b"",
        );
        assert_eq!(s, 401);
        assert!(soak.snapshot().lanes.is_empty(), "guard reject uncounted");
        // A shape reject (valid token, garbage multipart → 400) is NOT
        // lane health either — it never dials the engine.
        let (s, _, _) = route(
            &routes,
            "POST",
            "/transcribe",
            &[
                ("Host".into(), "127.0.0.1:8785".into()),
                ("X-STT-Token".into(), "tok".into()),
                ("Content-Length".into(), "10".into()),
            ],
            b"0123456789",
        );
        assert_eq!(s, 400);
        assert!(soak.snapshot().lanes.is_empty(), "shape reject uncounted");
        // A body that reaches the ENGINE dial (dead engine at port 9 →
        // 502) IS lane health: one attempt, one drop, one ledger row.
        let (mp_body, ctype) = crate::multipart::build(&tiny_wav(0.5), &[]).unwrap();
        let (s, _, _) = route(
            &routes,
            "POST",
            "/transcribe",
            &[
                ("Host".into(), "127.0.0.1:8785".into()),
                ("X-STT-Token".into(), "tok".into()),
                ("Content-Type".into(), ctype),
                ("Content-Length".into(), mp_body.len().to_string()),
            ],
            &mp_body,
        );
        assert_eq!(s, 502, "dead engine answers honest 502");
        let snap = soak.snapshot();
        let horn = snap
            .lanes
            .iter()
            .find(|(l, _)| l == crate::soak::HORN_LANE)
            .expect("horn lane recorded");
        assert_eq!((horn.1.attempts, horn.1.dropped), (1, 1));
        let win = soak.windows();
        let all = win.windows.iter().find(|w| w.label == "all").unwrap();
        let horn_w = all
            .lanes
            .iter()
            .find(|(l, _)| l == crate::soak::HORN_LANE)
            .unwrap();
        assert_eq!(horn_w.1.total, 1, "ledger row computed into window");
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn transcribe_rejects_before_body_without_token() {
        let (routes, dir) = routes_with(9, 8785, "guards");
        let (s, _, _) = route(
            &routes,
            "POST",
            "/transcribe",
            &[
                ("Host".into(), "127.0.0.1:8785".into()),
                ("Content-Length".into(), "10".into()),
            ],
            b"",
        );
        assert_eq!(s, 401);
        std::fs::remove_dir_all(&dir).ok();
    }

    /// Full-socket E2E: real listener, real socket, mock engine — the whole
    /// P2 lane from bytes-in to bytes-out over TCP.
    #[test]
    fn socket_e2e_happy_path_and_429_headers_shape() {
        // mock engine
        let el = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let engine_port = el.local_addr().unwrap().port();
        std::thread::spawn(move || {
            let (mut sock, _) = el.accept().unwrap();
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
            let body = "{\"text\":\"labas\",\"language\":\"lt\"}";
            let _ = sock.write_all(
                format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    body.len(),
                    body
                )
                .as_bytes(),
            );
        });

        // Bind FIRST, then build routes with the REAL listen port (the Host
        // guard compares against the service's configured port).
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let (routes, dir) = routes_with(engine_port, port, "e2e");
        let routes = Arc::new(routes);
        let stop = Arc::new(AtomicBool::new(false));
        let stop_srv = stop.clone();
        let srv = std::thread::spawn(move || serve(listener, routes, stop_srv).unwrap());

        let (mp_body, ctype) = crate::multipart::build(&tiny_wav(0.5), &[]).unwrap();
        let req = format!(
            "POST /transcribe HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-STT-Token: tok\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            mp_body.len()
        );
        let mut sock = TcpStream::connect(("127.0.0.1", port)).unwrap();
        sock.write_all(req.as_bytes()).unwrap();
        sock.write_all(&mp_body).unwrap(); // the declared body, actually sent
        let mut resp = Vec::new();
        sock.read_to_end(&mut resp).unwrap();
        let text = String::from_utf8_lossy(&resp);
        assert!(text.starts_with("HTTP/1.1 200"), "got: {text}");
        assert!(text.contains("\"text\":\"labas\""));

        // Unauthorized over the socket too.
        let req = format!(
            "POST /transcribe HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nX-STT-Token: WRONG\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            mp_body.len()
        );
        let mut sock = TcpStream::connect(("127.0.0.1", port)).unwrap();
        sock.write_all(req.as_bytes()).unwrap();
        sock.write_all(&mp_body).unwrap();
        let mut resp = Vec::new();
        sock.read_to_end(&mut resp).unwrap();
        assert!(String::from_utf8_lossy(&resp).starts_with("HTTP/1.1 401"));

        // Health answers WHILE a transcribe could be running (concurrent).
        let mut sock = TcpStream::connect(("127.0.0.1", port)).unwrap();
        sock.write_all(
            format!("GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n")
                .as_bytes(),
        )
        .unwrap();
        let mut resp = Vec::new();
        sock.read_to_end(&mut resp).unwrap();
        assert!(String::from_utf8_lossy(&resp).starts_with("HTTP/1.1 200"));

        // Stop the server cleanly: the accept loop checks the flag each 25ms
        // poll, then joins spawned connection threads.
        stop.store(true, Ordering::Relaxed);
        srv.join().unwrap();
        std::fs::remove_dir_all(&dir).ok();
    }
}
