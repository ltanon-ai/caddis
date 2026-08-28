//! prober_tests.rs — the stub-server FIXTURE TABLE (brief §6): one local
//! 127.0.0.1 HTTP server per status class + the transport-failure shapes.
//! All fixtures dial PLAIN http (the stub path); the schannel half is
//! Windows-only and live-proven in slice C (first live rotation), not
//! testable against a local stub without a TLS endpoint.

use super::probe;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

fn fast_cfg() -> super::ProbeCfg {
    super::ProbeCfg {
        connect_timeout: Duration::from_secs(2),
        total_timeout: Duration::from_secs(2),
    }
}

/// Spawn a one-connection stub server; returns (base_url, channel with the
/// raw request bytes the server received).
fn spawn_once<F>(handler: F) -> (String, mpsc::Receiver<Vec<u8>>)
where
    F: FnOnce(std::net::TcpStream) + Send + 'static,
{
    let listener = TcpListener::bind("127.0.0.1:0").expect("bind stub");
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let (mut sock, _) = listener.accept().expect("accept");
        let mut req = Vec::new();
        let mut tmp = [0u8; 8 * 1024];
        // Read until the head terminator (requests are tiny).
        loop {
            match sock.read(&mut tmp) {
                Ok(0) => break,
                Ok(n) => {
                    req.extend_from_slice(&tmp[..n]);
                    if req.windows(4).any(|w| w == b"\r\n\r\n") {
                        break;
                    }
                }
                Err(_) => break,
            }
        }
        let _ = tx.send(req);
        handler(sock);
    });
    (format!("http://127.0.0.1:{port}"), rx)
}

fn respond(mut sock: std::net::TcpStream, status_line: &str) {
    let head = format!("HTTP/1.1 {status_line}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
    let _ = sock.write_all(head.as_bytes());
    let _ = sock.flush();
}

fn port_of(url: &str) -> String {
    url.rsplit(':').next().unwrap_or("").to_string()
}

// ---------------------------------------------------------------------------
// The fixture table: one row per status class of the ruled map (§5.3).
// ---------------------------------------------------------------------------

#[test]
fn status_class_table() {
    // (code, reason) — every class the rotate status map branches on, plus
    // an unlisted 4xx/5xx that must still surface its code (transient).
    let table: &[(u16, &str)] = &[
        (200, "OK"),
        (401, "Unauthorized"),
        (402, "Payment Required"),
        (403, "Forbidden"),
        (429, "Too Many Requests"),
        (408, "Request Timeout"),
        (451, "Unavailable For Legal Reasons"),
        (500, "Internal Server Error"),
        (504, "Gateway Timeout"),
    ];
    for (code, reason) in table {
        let line = format!("{code} {reason}");
        let (url, _rx) = spawn_once(move |sock| respond(sock, &line));
        let out = probe(&url, "", &fast_cfg());
        assert_eq!(
            out.status,
            Some(*code),
            "status for {code}, error={:?}",
            out.error
        );
        assert!(out.error.is_none(), "no error for {code}: {:?}", out.error);
    }
}

#[test]
fn request_shape_and_auth_law() {
    // Key material lives in a temp file (the vault-path law); the probe
    // must send it as a bearer and ONLY when auth_path is non-blank.
    let key = "sekrit-key-123";
    let keyfile = std::env::temp_dir().join("caddis-prober-test-key");
    std::fs::write(&keyfile, format!("{key}\n")).expect("write keyfile");

    // With auth.
    {
        let (url, rx) = spawn_once(|sock| respond(sock, "200 OK"));
        let port = port_of(&url);
        let out = probe(&format!("{url}/v1"), keyfile.to_str().unwrap(), &fast_cfg());
        assert_eq!(out.status, Some(200));
        let req = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("request captured");
        let text = String::from_utf8(req).expect("utf8 request");
        assert!(
            text.starts_with("GET /v1/models HTTP/1.1\r\n"),
            "path law: {text}"
        );
        assert!(text.contains(&format!("Host: 127.0.0.1:{port}\r\n")));
        assert!(
            text.contains(&format!("Authorization: Bearer {key}\r\n")),
            "bearer law"
        );
        assert!(text.contains("Connection: close\r\n"), "close law");
    }
    // Without auth: NO Authorization line at all.
    {
        let (url, rx) = spawn_once(|sock| respond(sock, "200 OK"));
        let out = probe(&url, "   ", &fast_cfg());
        assert_eq!(out.status, Some(200));
        let req = rx
            .recv_timeout(Duration::from_secs(2))
            .expect("request captured");
        let text = String::from_utf8(req).expect("utf8 request");
        assert!(
            !text.contains("Authorization"),
            "blank auth probes unauthenticated"
        );
    }
    let _ = std::fs::remove_file(&keyfile);
}

#[test]
fn auth_file_defects_are_local_not_lane_verdicts() {
    let (url, _rx) = spawn_once(|sock| respond(sock, "200 OK"));
    // Missing file: transient class, honest reason, no status.
    let out = probe(&url, "Z:/definitely/not/here.key", &fast_cfg());
    assert_eq!(out.status, None);
    let reason = out.error.unwrap();
    assert!(reason.contains("auth file unreadable"), "reason: {reason}");
    let empty = std::env::temp_dir().join("caddis-prober-test-empty-key");
    std::fs::write(&empty, "").unwrap();
    let out = probe(&url, empty.to_str().unwrap(), &fast_cfg());
    assert!(out.error.unwrap().contains("is empty"));
    let _ = std::fs::remove_file(&empty);
}

#[test]
fn transport_failures() {
    // Server closes the connection before answering.
    {
        let (url, _rx) = spawn_once(drop);
        let out = probe(&url, "", &fast_cfg());
        assert_eq!(out.status, None);
        let reason = out.error.unwrap();
        assert!(
            reason.contains("closed") || reason.contains("reset"),
            "close class: {reason}"
        );
    }
    // Server accepts and never answers: the HARD total deadline fires.
    {
        let (url, _rx) = spawn_once(|_sock| thread::sleep(Duration::from_secs(5)));
        let t0 = std::time::Instant::now();
        let cfg = super::ProbeCfg {
            connect_timeout: Duration::from_secs(2),
            total_timeout: Duration::from_millis(800),
        };
        let out = probe(&url, "", &cfg);
        let dt = t0.elapsed();
        assert_eq!(out.status, None);
        // The deadline BOUNDS are the law (wording is OS-specific).
        assert!(dt >= Duration::from_millis(700), "deadline honored: {dt:?}");
        assert!(dt < Duration::from_secs(3), "deadline not exceeded: {dt:?}");
        let reason = out.error.expect("timeout carries a reason");
        assert!(!reason.is_empty());
    }
    // Malformed status line.
    {
        let (url, _rx) = spawn_once(|mut sock| {
            let _ = sock.write_all(b"GARBAGE\r\nContent-Length: 0\r\n\r\n");
            let _ = sock.flush();
        });
        let out = probe(&url, "", &fast_cfg());
        assert_eq!(out.status, None);
        assert!(out.error.unwrap().contains("malformed status line"));
    }
}

#[test]
fn slow_trickle_still_answers() {
    // Byte-by-byte status line: the remaining-deadline reads keep the
    // probe alive while budget remains (council risk gate: trickle must
    // not starve, deadline must bound).
    let (url, _rx) = spawn_once(|mut sock| {
        for b in b"HTTP/1.1 429 Too Many Requests\r\n\r\n" {
            let _ = sock.write_all(&[*b]);
            let _ = sock.flush();
            thread::sleep(Duration::from_millis(5));
        }
    });
    let out = probe(&url, "", &fast_cfg());
    assert_eq!(out.status, Some(429));
}

#[test]
fn head_cap_bounds_garbage_stream() {
    // An endless header stream with no blank line and no parseable status
    // must hit the cap and fail — never read forever.
    let (url, _rx) = spawn_once(|mut sock| {
        let pad = vec![b'A'; 8 * 1024];
        for _ in 0..8 {
            let _ = sock.write_all(&pad);
            let _ = sock.flush();
        }
        thread::sleep(Duration::from_secs(5));
    });
    let t0 = std::time::Instant::now();
    let out = probe(&url, "", &fast_cfg());
    assert_eq!(out.status, None);
    assert!(t0.elapsed() < Duration::from_secs(3));
    let reason = out.error.unwrap();
    assert!(
        reason.contains("exceeds cap") || reason.contains("malformed"),
        "bounded failure: {reason}"
    );
}

#[test]
fn url_law() {
    let cfg = fast_cfg();
    // No scheme / wrong scheme / userinfo refused — none of these dial.
    for (bad, needle) in [
        ("api.example.com/v1", "no scheme"),
        ("ftp://api.example.com", "unsupported scheme"),
        ("https://user:pass@api.example.com", "userinfo"),
    ] {
        let out = probe(bad, "", &cfg);
        assert_eq!(out.status, None, "{bad}");
        assert!(out.error.unwrap().contains(needle), "{bad}");
    }
}

#[test]
fn parse_status_unit() {
    assert_eq!(super::parse_status(b"HTTP/1.1 200 OK\r\n"), Some(200));
    assert_eq!(super::parse_status(b"HTTP/1.0 429\r\nStuff"), Some(429));
    assert_eq!(super::parse_status(b"HTTP/1.1 20 OK\r\n"), None);
    assert_eq!(super::parse_status(b"HTTP/1.1 2000 OK\r\n"), None);
    assert_eq!(super::parse_status(b"garbage"), None);
    assert_eq!(super::parse_status(b""), None);
}
