//! Live probe vs the REAL whisper engine on :8772 (--ignored by default).
//!
//! Run explicitly (sergeant tick / P4 soak tooling):
//! `cargo test -p caddis-voice --test live_probe -- --ignored --nocapture`
//!
//! SAFETY CONTRACT — these probes are READ-ONLY against the operator's live
//! STT stack (stt-daemon :8765 -> whisper-server :8772):
//! - NO spawn, NO kill, NO config write, NO port bind on 8772/8765.
//! - `live_adopt_identity` resolves the listener pid/image through netstat +
//!   tasklist (the same reads the daemon's own watchdog does every 2s).
//! - `live_silence_inference` sends ONE 0.3s silence WAV — byte-for-byte the
//!   daemon's own `available()` readiness probe (stt_gpu.py), small enough
//!   that the model answers in well under a second of GPU time.

#![cfg(windows)]

use caddis_voice::horn::{adopt_decision, EngineWorld, HornSettings, OsEngineWorld};
use caddis_voice::whisperc;

/// Silence WAV bytes: 16-bit PCM mono 16 kHz, `secs` long.
fn silence_wav(secs: f64) -> Vec<u8> {
    let rate: u32 = 16_000;
    let frames = (secs * rate as f64) as usize;
    let data_len = frames * 2;
    let mut b = Vec::with_capacity(44 + data_len);
    b.extend_from_slice(b"RIFF");
    b.extend_from_slice(&((36 + data_len) as u32).to_le_bytes());
    b.extend_from_slice(b"WAVE");
    b.extend_from_slice(b"fmt ");
    b.extend_from_slice(&16u32.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&1u16.to_le_bytes());
    b.extend_from_slice(&rate.to_le_bytes());
    b.extend_from_slice(&(rate * 2).to_le_bytes());
    b.extend_from_slice(&2u16.to_le_bytes());
    b.extend_from_slice(&16u16.to_le_bytes());
    b.extend_from_slice(b"data");
    b.extend_from_slice(&(data_len as u32).to_le_bytes());
    b.extend(std::iter::repeat_n(0u8, data_len));
    b
}

#[test]
#[ignore = "touches the live operator engine (read-only identity reads)"]
fn live_adopt_identity() {
    let s = HornSettings::default(); // engine_port 8772, whisper-server.exe
    let mut world = OsEngineWorld;
    let taken = world.port_taken(&s.engine_host, s.engine_port);
    println!("port {} taken: {taken}", s.engine_port);
    if !taken {
        // Honest finding: engine down (operator stopped it) — not a failure.
        println!("LIVE ENGINE DOWN — nothing to adopt; probe ends clean");
        return;
    }
    let pid = world.listening_pid(s.engine_port);
    let image = pid.and_then(|p| world.image_name(p));
    println!("listener pid={pid:?} image={image:?}");
    let decision = adopt_decision(pid, image.clone(), &s.engine_exe);
    match decision {
        Some((pid, image)) => println!("ADOPT-OK: pid={pid} image={image} (horn would supervise, never kill)"),
        None => println!(
            "ADOPT-REFUSED: port held by pid={pid:?} image={image:?} — NOT whisper-server.exe; horn would stay loud and untouched"
        ),
    }
    // The assertion is only that the identity reads WORK on this machine;
    // both adopt and refuse are legal outcomes (daemon-owned engine adopts;
    // anything else refuses).
    assert!(
        pid.is_some(),
        "port is taken but netstat found no listener pid — identity machinery broken"
    );
    assert!(
        image.is_some(),
        "tasklist could not resolve the listener image"
    );
}

#[test]
#[ignore = "one real inference against the live operator engine (daemon's own readiness probe)"]
fn live_silence_inference() {
    let s = HornSettings::default();
    let mut world = OsEngineWorld;
    if !world.port_taken(&s.engine_host, s.engine_port) {
        println!("LIVE ENGINE DOWN — nothing to probe; probe ends clean");
        return;
    }
    let wav = silence_wav(0.3);
    let t = whisperc::transcribe("127.0.0.1", s.engine_port, &wav, Some("lt"), false, 0.3)
        .expect("live engine must answer the daemon-shaped probe");
    println!(
        "text={:?} language={:?} duration={} segments={}",
        t.text,
        t.language,
        t.duration_s,
        t.segments.len()
    );
    // Silence may legitimately produce "" — the CONTRACT is a valid answer,
    // not a non-empty one. GA2 did its job the moment `text` was a string.
    assert!(t.duration_s > 0.0);
}
