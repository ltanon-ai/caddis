//! The caddis-voice executable.
//!
//! Two shapes share one binary (the play child spawns the SAME exe — the
//! organ playing its own output keeps the child sovereign: no Python, no
//! PortAudio; the daemon's play_proc.py contract ported verbatim):
//!
//! - `play-view <wav> <device>` — the KILLABLE PLAY CHILD (P3 slice b),
//!   spawned per attempt by [`caddis_voice::play::AudioOut`]. Exit
//!   contract 0/10/20/30/40; a play child that silently tolerated wrong
//!   argv would break that contract, so unknown argv fails closed with 2.
//! - `daemon [--home <dir>] [--no-horn]` — THE ORGAN (P4): load config
//!   (missing = embedded defaults, corrupt = LOUD refusal), take the port
//!   hard mutex, wire the REAL lanes from config (piper exe + per-voice
//!   ONNX models, AudioOut on the configured device), start horn
//!   supervision (adopt-don't-duplicate; `--no-horn` is the test posture)
//!   and serve `/health` + `/transcribe` + `/say` + `/earcon`.
//!
//!   Stop is PROCESS DEATH by design: the DeadManSwitch job reaps every
//!   child (renders, play views, an organ-spawned engine); an ADOPTED
//!   engine is never ours to kill. A graceful-stop route grows in P5 with
//!   the operator's panel, not before it has a user.

use caddis_voice::horn::{HornSettings, OsEngineWorld, Supervisor};
use caddis_voice::httpd::{self, OrganRoutes};
use caddis_voice::say::RenderLane;
use caddis_voice::{
    bind_exclusive, load_config, AudioOut, BreakerConfig, HealthState, HornService, PiperAdapter,
    PiperPaths, SayService, TokenGuard, VERSION,
};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() == 4 && args[1] == "play-view" {
        std::process::exit(caddis_voice::play::play_view(&args[2], &args[3]));
    }
    if args.len() >= 2 && args[1] == "daemon" {
        std::process::exit(run_daemon(&args[2..]));
    }
    eprintln!("usage: caddis-voice play-view <wav> <device>");
    eprintln!("       caddis-voice daemon [--home <dir>] [--no-horn]");
    std::process::exit(2);
}

/// The organ's state home: config, drop ledger. Default `~/.caddis/voice`.
fn default_home() -> PathBuf {
    std::env::var_os("USERPROFILE")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".caddis")
        .join("voice")
}

fn run_daemon(argv: &[String]) -> i32 {
    let mut home = default_home();
    let mut horn_on = true;
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "--home" if i + 1 < argv.len() => {
                home = PathBuf::from(&argv[i + 1]);
                i += 2;
            }
            "--no-horn" => {
                horn_on = false;
                i += 1;
            }
            other => {
                eprintln!("daemon: unknown argument {other:?}");
                return 2;
            }
        }
    }
    if let Err(e) = std::fs::create_dir_all(&home) {
        eprintln!("daemon: home {}: {e}", home.display());
        return 2;
    }

    // Config: missing file = embedded defaults (D6 boot law); a file that
    // exists but does not parse is a LOUD refusal — never a silent default
    // over an operator's broken edit.
    let cfg_path = home.join("organ.json");
    let (config, source) = match load_config(&cfg_path) {
        Ok(x) => x,
        Err(e) => {
            eprintln!("daemon: config {}: {e} — REFUSING to boot on defaults", cfg_path.display());
            return 2;
        }
    };

    // The port hard mutex: a conflict is the operator-visible signal, not
    // a reason to relocate (parallel-run = own port; P5 cutover = the old
    // daemon ports, after QQ1 retirement).
    let listener = match bind_exclusive(config.listen_port) {
        Ok(l) => l,
        Err(e) => {
            eprintln!("daemon: {e}");
            return 3;
        }
    };

    // Children die with the organ (P1 Job Objects law). Arming failure is
    // loud but not fatal: the organ still serves, degraded.
    match caddis_voice::DeadManSwitch::install() {
        Ok(_switch) => {}
        Err(e) => eprintln!("daemon: DeadManSwitch NOT armed ({e}) — children may outlive a crash"),
    }

    let health = Arc::new(HealthState::boot(
        "caddis-voice",
        VERSION,
        vec![config.listen_port],
    ));

    // The horn: adopt-don't-duplicate supervision of the whisper engine.
    // With the live engine on its port this ADOPTS (probes only); it only
    // ever spawns when the port is free. `--no-horn` keeps tests from
    // touching the GPU lane.
    let horn_settings = HornSettings::default();
    if horn_on {
        let settings = horn_settings.clone();
        let spawned = Arc::clone(&health.spawned_children);
        let res = std::thread::Builder::new()
            .name("caddis-voice-horn".into())
            .spawn(move || {
                let mut sup = Supervisor::new(settings.clone(), OsEngineWorld);
                loop {
                    for a in sup.tick().actions {
                        if a.starts_with("engine spawned") {
                            spawned.fetch_add(1, Ordering::Relaxed);
                        }
                        eprintln!("horn: {a}");
                    }
                    std::thread::sleep(Duration::from_millis(settings.poll_interval_ms));
                }
            });
        if let Err(e) = res {
            eprintln!("daemon: horn thread failed to spawn ({e}) — /transcribe still dials the engine directly");
        }
    }

    // The horn's mouth: engine facts + the shared token file (parallel-run
    // clients keep the X-STT-Token they already carry). `path` source
    // field stays OFF in v1 boot; uploads only.
    let horn = Arc::new(HornService::new(
        horn_settings.engine_host.clone(),
        horn_settings.engine_port,
        horn_settings.model_name.clone(),
        horn_settings.language.clone(),
        TokenGuard::new(&config.token_file),
        config.listen_port,
        Vec::new(),
    ));

    // The real render lanes from config. Piper with an empty exe = lane
    // simply not wired: every /say then drops LOUDLY (ledger + fail
    // chime) — an honest degraded boot, never a wrong-voice render.
    // leonas/ona (network LT) are admitted in the registry; their EdgeTts
    // lane lands in the next P4 slice.
    let mut lanes: Vec<Box<dyn RenderLane + Send>> = Vec::new();
    if config.piper.exe.is_empty() {
        eprintln!(
            "daemon: piper lane OFF (config piper.exe empty) — speech drops loudly until a lane is wired"
        );
    } else {
        let cap = config
            .registry
            .generator("piper")
            .map(|g| g.render_cap_ms)
            .unwrap_or(1500);
        let fallback = config.piper.voices.values().next().cloned().unwrap_or_default();
        let mut adapter = PiperAdapter::new(
            PiperPaths {
                exe: config.piper.exe.clone(),
                model: fallback,
                model_config: None,
            },
            cap,
        );
        for (voice, model) in &config.piper.voices {
            adapter = adapter.with_voice_model(voice, model, None);
        }
        eprintln!(
            "daemon: piper lane ON ({} voice model(s))",
            config.piper.voices.len()
        );
        lanes.push(Box::new(adapter));
    }

    let sink = AudioOut::new(&config.device_name);
    let say = Arc::new(SayService::start(
        config.clone(),
        lanes,
        Box::new(sink),
        Some(home.join("drop-ledger.jsonl")),
        BreakerConfig::default(),
    ));

    let routes = Arc::new(OrganRoutes {
        health,
        horn,
        say: Some(say),
    });
    let stop = Arc::new(AtomicBool::new(false));
    eprintln!(
        "daemon: caddis-voice {VERSION} on 127.0.0.1:{} (config: {}, horn: {}, home: {}) — stop is process death",
        config.listen_port,
        match source {
            caddis_voice::ConfigSource::File => "file",
            caddis_voice::ConfigSource::Embedded => "embedded default",
        },
        horn_on,
        home.display(),
    );
    match httpd::serve(listener, routes, stop) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("daemon: serve ended: {e}");
            4
        }
    }
}
