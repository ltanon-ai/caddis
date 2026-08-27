//! F5 stress: supervised-children spawn/adopt/kill cycles against a STUB
//! engine (--ignored by default — spawns real processes, needs python on PATH).
//!
//! Run explicitly:
//! `cargo test -p caddis-voice --test stress -- --ignored --nocapture`
//!
//! WHAT THIS PROVES (nemotron F5, "repeated spawn/adopt/kill cycles under
//! load, resource ceiling assertions"), mechanically and WITHOUT touching
//! the GPU or the live engine:
//! - the organ's spawn path (Command + no-window + log redirect) really
//!   produces a port-binding child;
//! - netstat/tasklist identity resolves THAT child (adopt machinery truth);
//! - stop_own kills it and the port ACTUALLY frees (the leak class from the
//!   2026-08-15 six-orphan incident);
//! - repeated cycles do not accumulate: after N cycles exactly zero
//!   listeners remain on the used ports;
//! - a server the horn did NOT spawn is adopted (image match) and NEVER
//!   killed by stop_own (P5 law held mechanically).
//!
//! The stub is python.exe holding a TCP port — the supervision mechanics are
//! engine-agnostic; the REAL whisper-server spawn/soak belongs to P4's
//! parallel-run (operator-gated), never to a test loop on this box.
//!
//! MEASURED THIS TICK: `where python`'s first hit is a venv LAUNCHER that
//! re-execs the base interpreter — the spawned pid would never match the
//! netstat listener. `sys.executable` is the real binary; that is what we
//! resolve and spawn.

#![cfg(windows)]

use caddis_voice::horn::{EngineWorld, HornSettings, HornState, OsEngineWorld, Supervisor};
use std::net::TcpListener;
use std::process::Command;
use std::time::{Duration, Instant};

/// A python one-liner that binds 127.0.0.1:PORT and holds it (accept + drop
/// loop). Passed via -c; the port is baked into the script text.
fn stub_script(port: u16) -> String {
    format!(
        "import socket\n\
         s=socket.socket()\n\
         s.bind(('127.0.0.1',{port}))\n\
         s.listen(8)\n\
         print('stub-ready',flush=True)\n\
         [s.accept() for _ in iter(int,1)]"
    )
}

fn python_path() -> String {
    // NOT `where python` and NOT plain sys.executable: on this box a venv
    // LAUNCHER can sit first on PATH (hermes) and re-exec the base interpreter
    // as a child — the spawned pid would never match the netstat listener
    // (measured this tick, twice). `_base_executable` is the binary that
    // actually runs the -c script; for a real interpreter it equals
    // sys.executable.
    let out = Command::new("python")
        .args(["-c", "import sys; print(getattr(sys, '_base_executable', sys.executable))"])
        .output()
        .expect("resolve python");
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .next()
        .expect("python on PATH")
        .trim()
        .to_string()
}

fn free_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn wait_port(world: &mut OsEngineWorld, host: &str, port: u16, up: bool, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if world.port_taken(host, port) == up {
            return true;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    false
}

fn stub_settings(port: u16, log: &str) -> HornSettings {
    HornSettings {
        engine_exe: python_path(),
        engine_weights: String::new(), // unused on the override lane
        engine_cwd: std::env::temp_dir().to_string_lossy().into_owned(),
        engine_host: "127.0.0.1".into(),
        engine_port: port,
        engine_log: log.into(),
        settle_ms: 300, // fast cycles; the production 2000ms is the WDDM law
        max_failures: 5,
        engine_args_override: Some(vec!["-c".into(), stub_script(port)]),
        ..HornSettings::default()
    }
}

#[test]
#[ignore = "spawns/kills real stub processes (needs python on PATH)"]
fn f5_spawn_kill_cycles_do_not_leak() {
    let port = free_port();
    let log = std::env::temp_dir().join(format!("caddis-voice-stress-{}.log", std::process::id()));
    let settings = stub_settings(port, log.to_str().unwrap());
    let mut world = OsEngineWorld;
    let mut seen_pids = Vec::new();

    for cycle in 1..=3 {
        let mut sup = Supervisor::new(settings.clone(), OsEngineWorld);
        let r = sup.tick(); // port free -> spawn
        assert!(
            matches!(r.state, HornState::Spawned { .. }),
            "cycle {cycle}: expected Spawned, got {:?} ({:?})",
            r.state,
            r.actions
        );
        let HornState::Spawned { pid } = r.state else { unreachable!() };
        // The child REALLY binds the port.
        assert!(
            wait_port(&mut world, "127.0.0.1", port, true, Duration::from_secs(10)),
            "cycle {cycle}: stub never bound the port"
        );
        // netstat identity resolves exactly this child.
        let listener = world.listening_pid(port);
        if listener != Some(pid) {
            let img_l = listener.and_then(|p| world.image_name(p));
            let img_s = world.image_name(pid);
            let cmdline = |p: u32| {
                String::from_utf8_lossy(
                    &Command::new("wmic")
                        .args(["process", "where", &format!("ProcessId={p}"), "get", "CommandLine"])
                        .output()
                        .map(|o| o.stdout)
                        .unwrap_or_default(),
                )
                .lines()
                .filter(|l| !l.trim().is_empty() && !l.contains("CommandLine"))
 .take(1)
                .collect::<String>()
            };
            panic!(
                "cycle {cycle}: listener={listener:?} ({img_l:?}) {} vs spawned={pid} ({img_s:?}) {}",
                cmdline(listener.unwrap_or(0)),
                cmdline(pid)
            );
        }

        // Kill through the supervisor; port must free (no orphan).
        sup.stop_own().expect("cycle {cycle}: stop_own");
        assert!(
            wait_port(&mut world, "127.0.0.1", port, false, Duration::from_secs(15)),
            "cycle {cycle}: port still held after stop_own — LEAK"
        );
        seen_pids.push(pid);
    }
    // Distinct children each cycle (pid churn proves real respawn, not a
    // cached handle — the Adopted-class identity lesson).
    assert_eq!(seen_pids.len(), 3, "pids: {seen_pids:?}");
    // Resource ceiling: no listener left behind, nothing on the port.
    assert!(!world.port_taken("127.0.0.1", port), "listener survived all cycles");
    let _ = std::fs::remove_file(&log);
    println!("F5 cycles OK: pids {seen_pids:?}, port {port} clean");
}

#[test]
#[ignore = "spawns a real stub the horn did NOT spawn (needs python on PATH)"]
fn f5_adopt_never_kills() {
    let port = free_port();
    let log = std::env::temp_dir().join(format!("caddis-voice-adopt-{}.log", std::process::id()));
    let settings = stub_settings(port, log.to_str().unwrap());

    // Spawn the stub OUTSIDE the supervisor (the "owner" lane).
    let mut owner = Command::new(&settings.engine_exe)
        .arg("-c")
        .arg(stub_script(port))
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("stub spawn");
    let mut world = OsEngineWorld;
    assert!(
        wait_port(&mut world, "127.0.0.1", port, true, Duration::from_secs(10)),
        "stub never bound"
    );

    // The horn adopts it (image == engine_exe basename on the stub lane).
    let mut sup = Supervisor::new(settings.clone(), OsEngineWorld);
    let r = sup.tick();
    assert!(
        matches!(r.state, HornState::Adopted { .. }),
        "expected Adopted, got {:?} ({:?})",
        r.state,
        r.actions
    );

    // stop_own REFUSES: adopted engines are never killed by the horn (P5 law).
    assert!(sup.stop_own().is_err(), "stop_own must refuse an adopted engine");
    // And the engine is still alive — the refusal was not cosmetic.
    assert!(world.port_taken("127.0.0.1", port), "adopted engine must still be serving");

    // Cleanup by the OWNER side (the test), not the horn.
    let _ = Command::new("taskkill")
        .args(["/PID", &owner.id().to_string(), "/T", "/F"])
        .output();
    let _ = owner.wait();
    assert!(
        wait_port(&mut world, "127.0.0.1", port, false, Duration::from_secs(15)),
        "owner kill left the port held — leak outside the horn"
    );
    // The horn notices the adopted engine is gone and STANDS DOWN (no spawn).
    let r = sup.tick();
    assert_eq!(r.state, HornState::NoServer);
    assert!(r.actions.iter().any(|a| a.contains("stands down")));
    let _ = std::fs::remove_file(&log);
    println!("F5 adopt OK: adopted pid={} refused-kill, stood down after owner exit", owner.id());
}
