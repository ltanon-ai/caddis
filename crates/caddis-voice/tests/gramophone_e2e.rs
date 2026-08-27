//! Integration: the P3 slice (c) gramophone E2E — queue → dispatch →
//! play child — over the REAL httpd socket, plus the earcon WAV → real
//! `play-view` child contract.
//!
//! Audio TRUTH (does it sound right) stays with the operator's ear
//! (R-F in-situ drills); here every child interaction is honest and
//! silent: the real child answers through its exit contract on a device
//! name that cannot exist, and the recording sink proves the shapes.

use caddis_voice::adapter::{AdapterErr, BreakerConfig, RenderedAudio};
use caddis_voice::config::OrganConfig;
use caddis_voice::registry::VoiceSpec;
use caddis_voice::say::{PlaySink, RenderLane};
use caddis_voice::sayd::SayService;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------
// fakes shared by the socket tests
// ---------------------------------------------------------------------

/// A lane whose render product is a REAL (silent) PCM16 WAV — whatever
/// the dispatcher does with it downstream parses.
struct SilentLane;
impl RenderLane for SilentLane {
    fn generator(&self) -> &str {
        "piper"
    }
    fn render(&self, _v: &VoiceSpec, _t: &str, _ls: f64) -> Result<RenderedAudio, AdapterErr> {
        let rate = 22_050u32;
        let frames = 2205usize; // 0.1 s mono silence
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        let data_len = (frames * 2) as u32;
        b.extend_from_slice(&(36 + data_len).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&rate.to_le_bytes());
        b.extend_from_slice(&(rate * 2).to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data_len.to_le_bytes());
        b.extend(std::iter::repeat_n([0u8, 0], frames).flatten());
        Ok(RenderedAudio {
            bytes: b,
            format: caddis_voice::adapter::AudioFormat::Wav,
            generator: "piper".into(),
            voice: "en_US-ryan".into(),
            elapsed_ms: 3,
            cap_ms: 1500,
            over_cap: false,
        })
    }
}

/// Recording sink: counts plays, remembers payload sizes (speech vs
/// earcon are audibly different SHAPES — here, different LENGTHS).
struct RecSink {
    plays: Arc<AtomicU32>,
    sizes: Arc<Mutex<Vec<usize>>>,
}
impl PlaySink for RecSink {
    fn play(&mut self, wav: &[u8]) -> bool {
        self.plays.fetch_add(1, Ordering::Relaxed);
        if let Ok(mut s) = self.sizes.lock() {
            s.push(wav.len());
        }
        true
    }
}

fn big_breaker() -> BreakerConfig {
    BreakerConfig {
        capacity: 100,
        refill_per_min: 100,
        cooldown_ms: 1_000,
    }
}

fn wait_counts(svc: &SayService, want_spoken: u64, deadline_s: u64) -> caddis_voice::sayd::SayCounts {
    let deadline = Instant::now() + Duration::from_secs(deadline_s);
    loop {
        let c = svc.counts();
        if c.spoken >= want_spoken || Instant::now() > deadline {
            return c;
        }
        std::thread::sleep(Duration::from_millis(25));
    }
}

// ---------------------------------------------------------------------
// 1. HTTP → queue → worker → dispatch → sink (real socket, real service)
// ---------------------------------------------------------------------

#[test]
fn say_over_real_socket_end_to_end() {
    let plays = Arc::new(AtomicU32::new(0));
    let sizes = Arc::new(Mutex::new(Vec::new()));
    let svc = Arc::new(SayService::start(
        OrganConfig::default(),
        vec![Box::new(SilentLane)],
        Box::new(RecSink {
            plays: plays.clone(),
            sizes: sizes.clone(),
        }),
        None,
        big_breaker(),
    ));

    // Health + horn routes untouched; bind a throwaway listener.
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let routes = Arc::new(caddis_voice::httpd::OrganRoutes {
        health: Arc::new(caddis_voice::HealthState::boot(
            "caddis-voice",
            caddis_voice::VERSION,
            vec![],
        )),
        horn: Arc::new(horn_stub()),
        say: Some(svc.clone()),
    });
    let stop = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let stop_srv = stop.clone();
    let srv = std::thread::spawn(move || caddis_voice::httpd::serve(listener, routes, stop_srv).unwrap());

    // /say one EN line: queued, then spoken by the worker.
    let body = br#"{"text":"End to end over the socket.","label":"sergeant"}"#;
    let resp = http(port, "POST", "/say", body);
    assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
    assert!(resp.contains("\"admission\":\"queued\""), "{resp}");
    let c = wait_counts(&svc, 1, 5);
    assert_eq!(c.spoken, 1, "worker spoke the submitted line: {c:?}");
    assert_eq!(c.queue_len, 0);

    // /earcon attention: 200, and the sink sees a SECOND, LARGER payload
    // (the synthesized motif: 48 kHz stereo, ~1.15 s — vs 0.1 s speech).
    let resp = http(port, "POST", "/earcon", br#"{"event":"attention"}"#);
    assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && plays.load(Ordering::Relaxed) < 2 {
        std::thread::sleep(Duration::from_millis(25));
    }
    let s = sizes.lock().unwrap().clone();
    assert_eq!(s.len(), 2, "speech + earcon reached the sink: {s:?}");
    assert!(s[1] > s[0] * 4, "the earcon is its own, larger payload: {s:?}");

    // A gated-confirm degrade over the socket: unknown label (no voice
    // set) + confirm path → degrade chime, nothing spoken beyond line 1.
    let resp = http(
        port,
        "POST",
        "/say",
        br#"{"text":"Patvirtinta.","label":"nobody","path":"confirm"}"#,
    );
    assert!(resp.starts_with("HTTP/1.1 200"), "{resp}");
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline && svc.counts().degraded < 1 {
        std::thread::sleep(Duration::from_millis(25));
    }
    let c = svc.counts();
    assert_eq!(c.degraded, 1, "R-B confirm degrade fired: {c:?}");
    assert_eq!(c.spoken, 1, "no substitute on the confirm path: {c:?}");

    stop.store(true, Ordering::Relaxed);
    srv.join().unwrap();
    // SayService::drop = stop(): final drain + stand-down ledgering.
}

fn http(port: u16, method: &str, path: &str, body: &[u8]) -> String {
    let mut sock = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let req = format!(
        "{method} {path} HTTP/1.1\r\nHost: 127.0.0.1:{port}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    );
    sock.write_all(req.as_bytes()).unwrap();
    sock.write_all(body).unwrap();
    let mut resp = Vec::new();
    sock.read_to_end(&mut resp).unwrap();
    String::from_utf8_lossy(&resp).into_owned()
}

/// A HornService pointing nowhere: /transcribe is NOT this test's lane;
/// it only needs to exist for the route table (never called).
fn horn_stub() -> caddis_voice::HornService {
    let dir = std::env::temp_dir().join(format!("caddis-e2e-horn-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let tf = dir.join("token.txt");
    std::fs::write(&tf, "tok").unwrap();
    caddis_voice::HornService::new(
        "127.0.0.1".into(),
        9,
        "large-v3".into(),
        Some("lt".into()),
        caddis_voice::TokenGuard::new(&tf),
        0,
        vec![],
    )
}

// ---------------------------------------------------------------------
// 2. earcon WAV → the REAL play-view child (silent contract)
// ---------------------------------------------------------------------

#[cfg(windows)]
#[test]
fn earcon_wav_through_real_play_child_is_exit_20() {
    use std::process::Command;
    let set = caddis_voice::EarconSet::default();
    let motif = set.earcon_for("attention").expect("attention mapped");
    let wav = caddis_voice::synth_earcon_wav(motif, caddis_voice::EARCON_SAMPLE_RATE);
    let dir = std::env::temp_dir().join("caddis-earcon-e2e");
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("attention.wav");
    std::fs::write(&path, &wav).unwrap();
    // A wav that PARSES (synth output) + a device that cannot exist:
    // the child answers NO_VIEW — proving the synth product is exactly
    // what the play child consumes, without a sound.
    let st = Command::new(env!("CARGO_BIN_EXE_caddis-voice"))
        .args(["play-view", path.to_str().unwrap(), "no-such-device-caddis-test"])
        .output()
        .unwrap();
    assert_eq!(
        st.status.code(),
        Some(20),
        "synth WAV parses; exact-name miss => EXIT_NO_VIEW"
    );
}

// ---------------------------------------------------------------------
// 3. queue → dispatch → the REAL child (via AudioOut argv to the real
//    exe, on a device that cannot exist): the drop is process_error and
//    NO chime fires (the sink is dead — the ledger is the truth).
// ---------------------------------------------------------------------

#[cfg(windows)]
#[test]
fn full_chain_real_child_drop_is_loud_and_chimeless() {
    let audio = caddis_voice::AudioOut::new("no-such-device-caddis-test").with_child_argv(
        vec![
            env!("CARGO_BIN_EXE_caddis-voice").to_string(),
            "play-view".into(),
            "{WAV}".into(),
            "{DEVICE}".into(),
        ],
        10.0,
    );
    let mut svc = SayService::start(
        OrganConfig::default(),
        vec![Box::new(SilentLane)],
        Box::new(audio),
        None,
        big_breaker(),
    );
    let (adm, _) = svc.say("sergeant", "Real child chain.", true, 0, caddis_voice::SpeechPath::GeneralSpeech);
    assert!(matches!(adm, caddis_voice::Admission::Queued));
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && svc.counts().dropped < 1 {
        std::thread::sleep(Duration::from_millis(50));
    }
    let c = svc.counts();
    assert_eq!(c.dropped, 1, "the real child honestly failed: {c:?}");
    assert_eq!(c.spoken, 0);
    assert_eq!(c.earcons_played, 0, "a dead sink fires NO chime");
    svc.stop();
}
