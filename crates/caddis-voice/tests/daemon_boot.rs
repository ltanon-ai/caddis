//! E2E of the daemon bin (P4): boot the REAL binary against a temp home
//! and prove the boot contract end-to-end — port mutex, /health surface,
//! /say admission, the token guard chain, and the loud refusals (a config
//! that exists but does not parse NEVER silently boots on defaults; a
//! second organ on a held port exits loud, it does not relocate).
//!
//! `--no-horn` everywhere: tests never touch the GPU engine lane. The say
//! tests point `device_name` at a nonexistent device so the full chain
//! (admit → dispatch → no lane → drop ledger) runs SILENTLY — a play-view
//! open failure is a `process_error`, and process errors fire no chime.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

/// Minimal HTTP/1.1 one-shot client (Connection: close servers only).
fn http(port: u16, req: &str) -> std::io::Result<(u16, String)> {
    let mut s = TcpStream::connect(("127.0.0.1", port))?;
    s.set_read_timeout(Some(Duration::from_secs(10)))?;
    s.write_all(req.as_bytes())?;
    let mut buf = Vec::new();
    s.read_to_end(&mut buf)?;
    let text = String::from_utf8_lossy(&buf).to_string();
    let status = text
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .unwrap_or(0);
    Ok((status, text))
}

/// A config document derived from the embedded default: this port, a temp
/// token file, a nonexistent audio device (silent drops), no piper exe.
fn write_config(home: &Path, port: u16) -> PathBuf {
    let token = home.join("token.txt");
    std::fs::write(&token, "e2e-token").unwrap();
    let doc = caddis_voice::DEFAULT_CONFIG_JSON
        .replace("\"listen_port\": 8768", &format!("\"listen_port\": {port}"))
        .replace(
            "C:/Users/ashpac/stt-daemon/stt-token.txt",
            &token.to_string_lossy().replace('\\', "/"),
        )
        .replace("\"device_name\": \"default\"", "\"device_name\": \"caddis-e2e-no-device\"");
    let path = home.join("organ.json");
    std::fs::write(&path, doc).unwrap();
    path
}

struct Daemon {
    child: Child,
    port: u16,
    home: PathBuf,
}

impl Drop for Daemon {
    fn drop(&mut self) {
        // Process death is the designed stop path; the job object reaps.
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn boot(home: &Path) -> Child {
    Command::new(env!("CARGO_BIN_EXE_caddis-voice"))
        .arg("daemon")
        .arg("--home")
        .arg(home)
        .arg("--no-horn")
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap()
}

fn wait_healthy(port: u16) {
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        if Instant::now() > deadline {
            panic!("daemon never served /health on port {port}");
        }
        if let Ok((200, body)) = http(
            port,
            "GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n",
        ) {
            assert!(body.contains("\"organ\":\"caddis-voice\""), "{body}");
            assert!(
                body.contains(&format!("\"ports_held\":[{port}]")),
                "{body}"
            );
            assert!(body.contains("\"spawned_children\":0"), "{body}");
            return;
        }
        std::thread::sleep(Duration::from_millis(150));
    }
}

/// Wait for exit with a deadline; returns (code, collected stderr).
fn finish(child: Child, secs: u64) -> (Option<i32>, String) {
    let mut child = child;
    let deadline = Instant::now() + Duration::from_secs(secs);
    loop {
        assert!(Instant::now() < deadline, "daemon did not exit in {secs}s");
        if child.try_wait().unwrap().is_some() {
            let mut err = String::new();
            if let Some(mut s) = child.stderr.take() {
                let _ = s.read_to_string(&mut err);
            }
            let code = child.wait().unwrap().code();
            return (code, err);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
}

#[test]
fn daemon_boots_serves_and_guards() {
    let home = std::env::temp_dir().join(format!("caddis-voice-e2e-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let port = free_port();
    write_config(&home, port);
    let child = boot(&home);
    let d = Daemon {
        child,
        port,
        home: home.clone(),
    };
    wait_healthy(d.port);

    // /say admits a line onto the queue (no piper lane wired: the drop is
    // asynchronous, ledgered, and inaudible on the fake device).
    let say_body = r#"{"text":"organ boot e2e","label":"e2e"}"#;
    let (st, body) = http(
        d.port,
        &format!(
            "POST /say HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
            say_body.len(),
            say_body
        ),
    )
    .unwrap();
    assert_eq!(st, 200, "{body}");
    assert!(body.contains("\"ok\":true"), "{body}");
    assert!(body.contains("\"admission\":\"queued\""), "{body}");

    // QQ4 soak instrument is live through the real bin: the /say drop
    // (no piper lane wired) lands asynchronously in /health's soak
    // section — per-lane counter + ledger window — the R-C/R-D contract.
    let deadline = Instant::now() + Duration::from_secs(10);
    let mut health = String::new();
    while Instant::now() < deadline {
        let (s, b) = http(d.port, "GET /health HTTP/1.1\r\nHost: x\r\nConnection: close\r\n\r\n").unwrap();
        assert_eq!(s, 200);
        health = b;
        if health.contains("\"dropped\":1") {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(health.contains("\"soak\""), "soak section present: {health}");
    assert!(health.contains("\"dropped\":1"), "lane drop counted: {health}");
    assert!(
        health.contains("\"windows\""),
        "availability windows present: {health}"
    );
    assert!(
        health.contains("\"detect\""),
        "detection telemetry present: {health}"
    );
    // The R-D ledger is a real file in the organ home, one row per
    // terminal outcome.
    let ledger = std::fs::read_to_string(home.join("soak-ledger.jsonl")).unwrap_or_default();
    assert!(
        ledger.contains("\"lane\":\"piper\""),
        "ledger row for the piper drop: {ledger}"
    );
    // bad JSON is a 400, not a crash.
    let (st, _) = http(
        d.port,
        "POST /say HTTP/1.1\r\nHost: x\r\nConnection: close\r\nContent-Length: 7\r\n\r\n{\"text\"\r\n",
    )
    .unwrap();
    assert_eq!(st, 400);

    // the guard chain is live through the real bin, in order: right Host +
    // real body + no token = 401 (zero-length bodies are 411'd before
    // routing; a wrong Host is 421 before the token).
    let host = format!("127.0.0.1:{}", d.port);
    let (st, body) = http(
        d.port,
        &format!(
            "POST /transcribe HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\nContent-Length: 5\r\n\r\nabcde"
        ),
    )
    .unwrap();
    assert_eq!(st, 401, "{body}");

    // known token passes the gate far enough to reach multipart parsing.
    let token = std::fs::read_to_string(home.join("token.txt")).unwrap();
    let req = format!(
        "POST /transcribe HTTP/1.1\r\nHost: {host}\r\nX-STT-Token: {token}\r\nConnection: close\r\nContent-Length: 5\r\n\r\nabcde"
    );
    let (st, body) = http(d.port, &req).unwrap();
    assert_eq!(st, 400, "{body}"); // valid token, garbage body → multipart 400

    drop(d); // kill + wait + clean home
}

#[test]
fn second_boot_on_held_port_fails_loud() {
    let home = std::env::temp_dir().join(format!("caddis-voice-e2e-mutex-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let port = free_port();
    write_config(&home, port);
    let first = Daemon {
        child: boot(&home),
        port,
        home: home.clone(),
    };
    wait_healthy(first.port);

    // The mutex must make the second organ EXIT LOUD with the port number,
    // never relocate (the ephemeral-fallback defect this law exists for).
    let second = boot(&home);
    let (code, err) = finish(second, 15);
    assert_eq!(code, Some(3), "{err}");
    assert!(err.contains(&port.to_string()), "{err}");
    assert!(err.to_lowercase().contains("held"), "{err}");

    drop(first);
}

#[test]
fn corrupt_config_refuses_to_boot() {
    let home = std::env::temp_dir().join(format!("caddis-voice-e2e-cfg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    std::fs::write(home.join("organ.json"), "{ definitely not json").unwrap();
    let child = boot(&home);
    let (code, err) = finish(child, 15);
    assert_eq!(code, Some(2), "{err}");
    assert!(err.contains("REFUSING"), "{err}");
    let _ = std::fs::remove_dir_all(&home);
}

#[test]
fn unknown_argv_fails_closed() {
    // A play child that silently tolerated wrong argv would break the
    // exit-code contract; the daemon shape holds the same line.
    let out = Command::new(env!("CARGO_BIN_EXE_caddis-voice"))
        .arg("definitely-not-a-subcommand")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("usage:"), "{err}");
}
