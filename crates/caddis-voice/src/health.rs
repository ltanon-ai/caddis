//! health.rs — `/health` with the VRAM capacity report (QQ2 law: the report
//! exists BEFORE any spawn can).
//!
//! The endpoint is intentionally boring: HTTP/1.1, GET /health only, one
//! JSON body, connection close. No framework, no threads-per-connection —
//! /health is polled, not browsed. The body carries the facts the operator
//! and the spawn-side (P2) both need: organ identity, uptime, the ports the
//! mutex actually holds, the VRAM snapshot, and `spawned_children` — the
//! counter that makes the QQ2 ordering OBSERVABLE: a fresh organ serves
//! `spawned_children: 0` alongside a real VRAM report; the report came
//! first, by construction of this slice.
//!
//! Request reading is capped (8 KiB head) and malformed requests get 400 —
//! health may be strict; it must never be fragile.

use crate::json::{self, Value};
use crate::mutex::bind_exclusive;
use crate::vram::VramReport;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Instant;

/// Cap on the request head we read (bytes). Anything bigger is not a health
/// probe; it gets the 400 and a closed socket.
const HEAD_CAP: usize = 8 * 1024;

/// Everything /health reports. Cheap to clone per-connection; the counters
/// are shared atomics so engine code (P2) increments them without touching
/// the server loop.
#[derive(Debug)]
pub struct HealthState {
    pub organ: &'static str,
    pub version: &'static str,
    pub started: Instant,
    /// The VRAM snapshot taken at boot (BEFORE any spawn — QQ2). A P2
    /// refresh API may replace it; it may never be silently absent.
    pub vram: VramReport,
    /// Ports the port mutex currently holds.
    pub ports: Vec<u16>,
    /// Children spawned by the organ so far (0 at boot, by definition).
    pub spawned_children: Arc<AtomicU64>,
}

impl HealthState {
    /// Boot state: VRAM probed now, counters zeroed.
    pub fn boot(organ: &'static str, version: &'static str, ports: Vec<u16>) -> Self {
        HealthState {
            organ,
            version,
            started: Instant::now(),
            vram: crate::vram::probe(),
            ports,
            spawned_children: Arc::new(AtomicU64::new(0)),
        }
    }

    /// The JSON body of GET /health. Field order is the readable contract.
    pub fn body(&self) -> String {
        let v = Value::Obj(vec![
            ("organ".into(), Value::Str(self.organ.into())),
            ("version".into(), Value::Str(self.version.into())),
            (
                "uptime_ms".into(),
                Value::Num(self.started.elapsed().as_millis() as u64 as f64),
            ),
            (
                "ports_held".into(),
                Value::Arr(self.ports.iter().map(|p| Value::Num(*p as f64)).collect()),
            ),
            ("spawned_children".into(), Value::Num(self.spawned_children.load(Ordering::Relaxed) as f64)),
            ("vram".into(), self.vram.to_value()),
        ]);
        json::to_string(&v)
    }
}

/// One parsed response, separated from the socket so routing is a pure,
/// table-testable function.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Response {
    pub status: u16,
    pub body: String,
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        405 => "Method Not Allowed",
        _ => "Internal Server Error",
    }
}

/// Route one request head ("METHOD SP PATH SP HTTP/x.y\r\n..."). Pure.
pub fn route(head: &[u8], state: &HealthState) -> Response {
    let text = match std::str::from_utf8(head) {
        Ok(t) => t,
        Err(_) => return Response { status: 400, body: error_body("request head is not UTF-8") },
    };
    let mut lines = text.split("\r\n");
    let request_line = match lines.next() {
        Some(l) if !l.is_empty() => l,
        _ => return Response { status: 400, body: error_body("empty request line") },
    };
    let mut parts = request_line.split(' ');
    let (method, path) = match (parts.next(), parts.next(), parts.next()) {
        (Some(m), Some(p), Some(_v)) => (m, p),
        _ => return Response { status: 400, body: error_body("malformed request line") },
    };
    match (method, path) {
        ("GET", "/health") => Response { status: 200, body: state.body() },
        ("GET", _) => Response { status: 404, body: error_body("only /health exists") },
        (_, "/health") => Response { status: 405, body: error_body("health is GET-only") },
        _ => Response { status: 404, body: error_body("only /health exists") },
    }
}

fn error_body(msg: &str) -> String {
    json::to_string(&Value::Obj(vec![("error".into(), Value::Str(msg.into()))]))
}

/// Serve on an ALREADY-BOUND listener until `stop` flips. The listener is
/// put in non-blocking mode and polled at 25ms — a health loop that burns a
/// core or blocks forever on accept would be its own defect.
pub fn serve(listener: TcpListener, state: Arc<HealthState>, stop: Arc<AtomicBool>) -> std::io::Result<()> {
    listener.set_nonblocking(true)?;
    while !stop.load(Ordering::Relaxed) {
        match listener.accept() {
            Ok((mut sock, _addr)) => {
                let head = read_head(&mut sock);
                let resp = match head {
                    Ok(h) => route(&h, &state),
                    Err(e) => Response { status: 400, body: error_body(&e) },
                };
                let _ = write_response(&mut sock, &resp); // a failed health write is a dead peer, not our failure
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            Err(e) => return Err(e),
        }
    }
    Ok(())
}

/// Read up to the head terminator, capped. `Err` is a 400 reason.
fn read_head(sock: &mut std::net::TcpStream) -> Result<Vec<u8>, String> {
    sock.set_read_timeout(Some(std::time::Duration::from_secs(2)))
        .map_err(|e| e.to_string())?;
    let mut buf = Vec::new();
    let mut chunk = [0u8; 512];
    loop {
        match sock.read(&mut chunk) {
            Ok(0) => return Err("connection closed before the request head ended".into()),
            Ok(n) => {
                buf.extend_from_slice(&chunk[..n]);
                if buf.windows(4).any(|w| w == b"\r\n\r\n") {
                    return Ok(buf);
                }
                if buf.len() > HEAD_CAP {
                    return Err("request head exceeds 8 KiB cap".into());
                }
            }
            Err(e) => return Err(format!("read: {e}")),
        }
    }
}

fn write_response(sock: &mut std::net::TcpStream, resp: &Response) -> std::io::Result<()> {
    let head = format!(
        "HTTP/1.1 {} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        resp.status,
        reason(resp.status),
        resp.body.len()
    );
    sock.write_all(head.as_bytes())?;
    sock.write_all(resp.body.as_bytes())?;
    sock.flush()
}

/// Bind the health port exclusively (the port mutex) and return the listener
/// together with the bound port. A convenience that keeps the QQ1/QQ2
/// pairing honest: the health port itself is mutex-held from the first
/// instant, because bind IS the claim.
pub fn bind_health_port(port: u16) -> Result<(TcpListener, u16), crate::mutex::PortMutexErr> {
    let l = bind_exclusive(port)?;
    let p = l.local_addr().map_err(|e| crate::mutex::PortMutexErr::Other {
        port,
        cause: e.to_string(),
    })?
    .port();
    Ok((l, p))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> HealthState {
        HealthState::boot("caddis-voice", crate::VERSION, vec![8790])
    }

    fn head(m: &str, p: &str) -> Vec<u8> {
        format!("{m} {p} HTTP/1.1\r\nHost: x\r\n\r\n").into_bytes()
    }

    #[test]
    fn routing_table() {
        let s = state();
        assert_eq!(route(&head("GET", "/health"), &s).status, 200);
        assert_eq!(route(&head("GET", "/gone"), &s).status, 404);
        assert_eq!(route(&head("POST", "/health"), &s).status, 405);
        assert_eq!(route(b"garbage", &s).status, 400);
        assert_eq!(route(b"", &s).status, 400);
        // Exact path only — /health?x is not /health.
        assert_eq!(route(&head("GET", "/health?x=1"), &s).status, 404);
    }

    #[test]
    fn health_body_carries_the_qq2_contract() {
        let s = state();
        let body = s.body();
        let v = json::parse(&body).expect("health body parses");
        assert_eq!(v.get("organ").and_then(Value::as_str), Some("caddis-voice"));
        assert_eq!(v.get("spawned_children").and_then(Value::as_f64), Some(0.0));
        assert!(v.get("vram").is_some(), "vram must be present at boot");
        assert_eq!(
            v.get("ports_held").and_then(Value::as_arr).map(|a| a.len()),
            Some(1)
        );
        // The counter is live: bump it, see it.
        s.spawned_children.store(3, Ordering::Relaxed);
        let v2 = json::parse(&s.body()).unwrap();
        assert_eq!(v2.get("spawned_children").and_then(Value::as_f64), Some(3.0));
    }

    #[test]
    fn real_socket_end_to_end() {
        // Ephemeral discovery is TEST-ONLY (the organ runtime never binds 0).
        let probe = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = probe.local_addr().unwrap().port();
        drop(probe);

        let (listener, port) = bind_health_port(port).expect("bind health port");
        let st = Arc::new(HealthState::boot("caddis-voice", crate::VERSION, vec![port]));
        let stop = Arc::new(AtomicBool::new(false));
        let s2 = Arc::clone(&st);
        let stop2 = Arc::clone(&stop);
        let server = std::thread::spawn(move || serve(listener, s2, stop2));

        let mut sock = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        sock.write_all(b"GET /health HTTP/1.1\r\nHost: organ\r\n\r\n").unwrap();
        let mut raw = String::new();
        sock.read_to_string(&mut raw).unwrap();
        drop(sock);

        let (h, b) = raw.split_once("\r\n\r\n").expect("head/body split");
        assert!(h.starts_with("HTTP/1.1 200 OK"), "{h}");
        assert!(h.contains("Content-Type: application/json"), "{h}");
        let v = json::parse(b).expect("body is JSON");
        assert_eq!(v.get("organ").and_then(Value::as_str), Some("caddis-voice"));
        assert_eq!(v.get("spawned_children").and_then(Value::as_f64), Some(0.0));

        // A 404 path through the real socket too.
        let mut sock = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        sock.write_all(b"GET /nope HTTP/1.1\r\nHost: organ\r\n\r\n").unwrap();
        let mut raw = String::new();
        sock.read_to_string(&mut raw).unwrap();
        assert!(raw.starts_with("HTTP/1.1 404"), "{raw}");

        stop.store(true, Ordering::Relaxed);
        server.join().expect("serve exits cleanly").expect("serve returns Ok");
    }
}
