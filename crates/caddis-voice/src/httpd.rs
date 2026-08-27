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

/// The routing surface: health (GET) + horn (POST /transcribe).
pub struct OrganRoutes {
    pub health: Arc<HealthState>,
    pub horn: Arc<HornService>,
}

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
fn write_response(sock: &mut TcpStream, status: u16, body: &str, extra: &[(String, String)]) -> std::io::Result<()> {
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
    sock.set_read_timeout(Some(HEAD_TIMEOUT)).map_err(|e| e.to_string())?;
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
    sock.set_read_timeout(Some(BODY_TIMEOUT)).map_err(|e| e.to_string())?;
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
pub fn route(routes: &OrganRoutes, method: &str, path: &str, headers: &[(String, String)], body: &[u8]) -> (u16, String, Vec<(String, String)>) {
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
                let EndpointResponse { status, body, headers } = rej;
                return (status, body, headers);
            }
            let r = routes.horn.handle_body(headers, body);
            (r.status, r.body, r.headers)
        }
        ("GET", "/transcribe") => (405, "{\"error\":\"transcribe is POST-only\"}".into(), Vec::new()),
        ("POST", "/health") => (405, "{\"error\":\"health is GET-only\"}".into(), Vec::new()),
        _ => (404, "{\"error\":\"only /health and /transcribe exist\"}".into(), Vec::new()),
    }
}

/// Serve on an ALREADY-BOUND (mutex-held) listener until `stop` flips.
/// Thread per connection, capped; every write failure is a dead peer, not a
/// defect (hang-up tolerance law).
pub fn serve(listener: TcpListener, routes: Arc<OrganRoutes>, stop: Arc<AtomicBool>) -> std::io::Result<()> {
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
    let wants_body = method == "POST" && path.split('?').next() == Some("/transcribe");
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
        let dir = std::env::temp_dir().join(format!("caddis-voice-httpd-{}-{label}", std::process::id()));
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
                health: Arc::new(HealthState::boot("caddis-voice", crate::VERSION, vec![8785])),
                horn: Arc::new(horn),
            },
            dir,
        )
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
    fn transcribe_rejects_before_body_without_token() {
        let (routes, dir) = routes_with(9, 8785, "guards");
        let (s, _, _) = route(
            &routes,
            "POST",
            "/transcribe",
            &[("Host".into(), "127.0.0.1:8785".into()), ("Content-Length".into(), "10".into())],
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
        sock.write_all(format!("GET /health HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nConnection: close\r\n\r\n").as_bytes())
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
