//! horn.rs — the LISTENING HORN supervisor: adopt-don't-duplicate lifecycle
//! for whisper-server.exe (P2 core).
//!
//! Every rule here was paid for on this machine, not invented:
//!
//! - **Liveness = port bound, not child-alive.** A TCP connect probe survives
//!   supervisor restarts; `poll()` on a recycled pid would happily report a
//!   dead server as alive and the operator's dictation would die silently.
//! - **Adopt-don't-duplicate, with identity.** Six leaked whisper servers were
//!   found alive on 2026-08-15. When the engine port is already held, the horn
//!   resolves the LISTENING pid (netstat) and its image (tasklist) and adopts
//!   it ONLY if the image is whisper-server.exe. A stranger on the port is
//!   NEVER adopted and NEVER killed — `StrangerOnPort` is a loud state.
//! - **An adopted server is never killed by the horn.** Adoption means
//!   supervision + proxying. The live engine belongs to the operator's
//!   running daemon; retirement is the P5 cutover law (operator-gated),
//!   not a supervisor reflex.
//! - **Spawn: settle before start.** After a death/kill the horn waits for
//!   the port to free PLUS a settle window (the post-kill WDDM reclaim race
//!   that once hung a model load). The QQ2 VRAM snapshot is taken and
//!   attached to every spawn attempt — measured-first doctrine: DXGI's Desc1
//!   reports TOTAL, not free, so a free-VRAM gate would invent a number;
//!   the snapshot is telemetry, the settle is the gate. (If soak shows OOM
//!   races anyway, a perf-counter free-VRAM poll is the P4 hardening.)
//! - **Backoff + blocked.** Repeated spawn failures back off exponentially;
//!   after `max_failures` the supervisor stops thrashing and parks in
//!   `Blocked` until [`Supervisor::unblock`] — the surface shows it, the
//!   operator (or P4 tooling) clears it. A blocked horn is honest; a
//!   spawn-looping horn is noise.
//!
//! Side effects are isolated behind [`EngineWorld`] so the whole state
//! machine is table-testable against a fake world; [`OsEngineWorld`] is the
//! thin real one (connect probe, netstat, tasklist, Command spawn).

use crate::vram;
use std::process::Command;
use std::time::{Duration, Instant};

/// Engine spawn flag: no console window (the tray-era daemon law; a sovereign
/// organ has the world UI, never stray consoles).
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Backoff base: 2^(failures-1) seconds, capped.
const BACKOFF_CAP: Duration = Duration::from_secs(30);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HornSettings {
    /// Path to whisper-server.exe.
    pub engine_exe: String,
    /// Path to the ggml weights file.
    pub engine_weights: String,
    /// Working directory for the engine (Vulkan shaders live beside it).
    pub engine_cwd: String,
    /// Host the engine binds / the horn dials.
    pub engine_host: String,
    /// Engine port. Default 8772 = the live operator port: the horn ADOPTS,
    /// it does not double-spawn.
    pub engine_port: u16,
    /// Decode threads (daemon-proven 8 on this box).
    pub threads: u32,
    /// Language hint (`-l`), e.g. "lt". None = server default.
    pub language: Option<String>,
    /// What the /transcribe response reports as the model.
    pub model_name: String,
    /// Log file for an organ-spawned engine (NEVER the live daemon's
    /// whisper-server.log — parallel-run writes are separate files).
    pub engine_log: String,
    /// Cold-load grace: 3 GB of weights onto the card, worst case (daemon law).
    pub start_grace_s: u64,
    /// Settle window after a death before a respawn attempt.
    pub settle_ms: u64,
    /// Supervision poll interval.
    pub poll_interval_ms: u64,
    /// Consecutive failures before Blocked.
    pub max_failures: u32,
    /// TEST/STUB lane ONLY: replaces the whisper argv template wholesale.
    /// Production leaves None — the template is the daemon-proven shape,
    /// and anything else talking to the GPU must go through review.
    pub engine_args_override: Option<Vec<String>>,
}

impl Default for HornSettings {
    fn default() -> Self {
        HornSettings {
            engine_exe: r"E:\Wok\models\stt\whisper-vulkan\whisper-server.exe".into(),
            engine_weights: r"E:\Wok\models\stt\whisper-vulkan\ggml-large-v3.bin".into(),
            engine_cwd: r"E:\Wok\models\stt\whisper-vulkan".into(),
            engine_host: "127.0.0.1".into(),
            engine_port: 8772,
            threads: 8,
            language: Some("lt".into()),
            model_name: "large-v3".into(),
            engine_log: r"E:\Wok\models\stt\whisper-vulkan\whisper-server-organ.log".into(),
            start_grace_s: 90,
            settle_ms: 2000,
            poll_interval_ms: 2000,
            max_failures: 5,
            engine_args_override: None,
        }
    }
}

/// What the horn is currently supervising (or failing to).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HornState {
    /// Nothing on the port, no failures pending.
    NoServer,
    /// Supervising an engine we spawned.
    Spawned { pid: u32 },
    /// Supervising an engine we adopted (never killed by us).
    Adopted { pid: u32, image: String },
    /// Port held by something that is NOT our engine. Loud, untouched.
    StrangerOnPort { pid: u32, image: String },
    /// Spawn failures exhausted the budget; parked until unblock().
    Blocked { failures: u32 },
}

/// One supervision tick's outcome — actions taken, for the caller to log.
#[derive(Debug, Clone, PartialEq)]
pub struct TickReport {
    pub state: HornState,
    pub actions: Vec<String>,
}

/// The world the supervisor acts on. Real impl: TCP/netstat/tasklist/spawn.
pub trait EngineWorld {
    fn port_taken(&mut self, host: &str, port: u16) -> bool;
    fn listening_pid(&mut self, port: u16) -> Option<u32>;
    fn image_name(&mut self, pid: u32) -> Option<String>;
    /// Spawn the engine with `settings`; return its pid.
    fn spawn_engine(&mut self, settings: &HornSettings) -> Result<u32, String>;
}

// ---------------------------------------------------------------------------
// OS implementation
// ---------------------------------------------------------------------------

/// The real world: connect probes, netstat/tasklist identity, Command spawn.
pub struct OsEngineWorld;

impl EngineWorld for OsEngineWorld {
    fn port_taken(&mut self, host: &str, port: u16) -> bool {
        std::net::TcpStream::connect((host, port))
            .map(|s| {
                let _ = s.set_read_timeout(Some(Duration::from_millis(500)));
                true
            })
            .is_ok()
    }

    fn listening_pid(&mut self, port: u16) -> Option<u32> {
        let out = Command::new("netstat")
            .args(["-ano", "-p", "TCP"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() >= 5 && parts[0] == "TCP" && parts[3] == "LISTENING" {
                // Colon-anchored port parse: ":18772" must never match 8772.
                let local_port = parts[1].rsplit(':').next()?.parse::<u16>().ok()?;
                if local_port == port {
                    return parts[4].parse::<u32>().ok();
                }
            }
        }
        None
    }

    fn image_name(&mut self, pid: u32) -> Option<String> {
        let out = Command::new("tasklist")
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .output()
            .ok()?;
        let text = String::from_utf8_lossy(&out.stdout);
        let first = text.lines().next()?.trim();
        if first.is_empty() || first.contains("No tasks") {
            return None;
        }
        let name = first.split(',').next()?.trim().trim_matches('"');
        if name.is_empty() {
            None
        } else {
            Some(name.to_string())
        }
    }

    fn spawn_engine(&mut self, s: &HornSettings) -> Result<u32, String> {
        if !std::path::Path::new(&s.engine_exe).is_file() {
            return Err(format!("engine exe missing: {}", s.engine_exe));
        }
        let log = std::fs::File::create(&s.engine_log)
            .map_err(|e| format!("cannot open engine log {}: {e}", s.engine_log))?;
        let log_err = log
            .try_clone()
            .map_err(|e| format!("cannot clone engine log handle: {e}"))?;
        let mut cmd = Command::new(&s.engine_exe);
        match &s.engine_args_override {
            // TEST/STUB lane (see HornSettings): the override IS the argv.
            Some(argv) => {
                cmd.args(argv);
            }
            None => {
                if !std::path::Path::new(&s.engine_weights).is_file() {
                    return Err(format!("weights missing: {}", s.engine_weights));
                }
                cmd.args([
                    "-m",
                    &s.engine_weights,
                    "--host",
                    &s.engine_host,
                    "--port",
                    &s.engine_port.to_string(),
                    "-t",
                    &s.threads.to_string(),
                    // -sow + -ml 0: never split a Lithuanian word mid-token;
                    // segments end where sentences do (daemon-proven,
                    // operator-verified).
                    "-sow",
                    "-ml",
                    "0",
                ]);
                if let Some(lang) = &s.language {
                    cmd.args(["-l", lang]);
                }
            }
        }
        cmd.current_dir(&s.engine_cwd);
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        cmd.stdout(std::process::Stdio::from(log));
        cmd.stderr(std::process::Stdio::from(log_err));
        // NOTE: stdin is inherited-null by default; the engine never reads it.
        let child = cmd.spawn().map_err(|e| format!("spawn: {e}"))?;
        // The organ's DeadManSwitch job (armed at boot) claims the child
        // automatically — if the organ dies, the kernel reaps the engine.
        Ok(child.id())
    }
}

// ---------------------------------------------------------------------------
// Supervisor
// ---------------------------------------------------------------------------

/// Backoff before spawn attempt number `failures+1`.
pub fn backoff_for(failures: u32) -> Duration {
    let shift = failures.min(5); // 1,2,4,8,16 then capped
    Duration::from_secs(1u64 << shift).min(BACKOFF_CAP)
}

/// The adopt-don't-duplicate gate, as a pure decision over identity facts.
/// `None` = do not adopt (stranger or unknown); caller stays loud, never lethal.
pub fn adopt_decision(
    listening_pid: Option<u32>,
    image: Option<String>,
    engine_exe: &str,
) -> Option<(u32, String)> {
    let pid = listening_pid?;
    let image = image?;
    let expected = engine_exe
        .rsplit(['\\', '/'])
        .next()
        .unwrap_or(engine_exe)
        .to_ascii_lowercase();
    if image.to_ascii_lowercase() == expected {
        Some((pid, image))
    } else {
        None
    }
}

pub struct Supervisor<W: EngineWorld> {
    pub settings: HornSettings,
    world: W,
    state: HornState,
    failures: u32,
    /// When the current healthy relationship began (sustained health clears
    /// the failure count, daemon HEALTHY_RESET law).
    healthy_since: Option<Instant>,
    /// Earliest next spawn attempt (backoff / settle).
    next_spawn_at: Option<Instant>,
}

impl<W: EngineWorld> Supervisor<W> {
    pub fn new(settings: HornSettings, world: W) -> Self {
        Supervisor {
            settings,
            world,
            state: HornState::NoServer,
            failures: 0,
            healthy_since: None,
            next_spawn_at: None,
        }
    }

    pub fn state(&self) -> &HornState {
        &self.state
    }

    /// Operator/P4 action: leave Blocked and try again.
    pub fn unblock(&mut self) {
        if matches!(self.state, HornState::Blocked { .. }) {
            self.failures = 0;
            self.state = HornState::NoServer;
            self.next_spawn_at = None;
        }
    }

    /// One reconcile pass. Pure-ish: every OS touch goes through
    /// [`EngineWorld`]; every conclusion lands in the report.
    pub fn tick(&mut self) -> TickReport {
        let mut actions = Vec::new();
        let taken = self.world.port_taken(&self.settings.engine_host, self.settings.engine_port);

        if taken {
            match &self.state {
                HornState::Spawned { .. } | HornState::Adopted { .. } => {
                    // Healthy. Sustained health (10s) clears the failure count.
                    let since = *self.healthy_since.get_or_insert_with(Instant::now);
                    if self.failures > 0 && since.elapsed() >= Duration::from_secs(10) {
                        actions.push("healthy 10s: failure count cleared".into());
                        self.failures = 0;
                    }
                }
                _ => {
                    // Port taken and we hold no relationship: ADOPT or refuse.
                    let pid = self.world.listening_pid(self.settings.engine_port);
                    let image = pid.and_then(|p| self.world.image_name(p));
                    match adopt_decision(pid, image.clone(), &self.settings.engine_exe) {
                        Some((pid, image)) => {
                            self.state = HornState::Adopted { pid, image: image.clone() };
                            self.healthy_since = Some(Instant::now());
                            actions.push(format!("adopted engine pid={pid} ({image}) — supervised, never killed by the horn"));
                        }
                        None => {
                            let (pid, image) = match (pid, image) {
                                (Some(p), Some(i)) => (p, i),
                                _ => (0, "unknown".into()),
                            };
                            self.state = HornState::StrangerOnPort { pid, image: image.clone() };
                            actions.push(format!(
                                "port {} held by pid={pid} ({image}) — NOT ours, NOT adopted, NOT touched",
                                self.settings.engine_port
                            ));
                        }
                    }
                }
            }
            return TickReport { state: self.state.clone(), actions };
        }

        // Port free.
        match &self.state {
            HornState::Spawned { pid } => {
                actions.push(format!("engine pid={pid} died (port free)"));
                self.enter_failure_state(&mut actions);
            }
            HornState::Adopted { pid, image } => {
                // The adopted engine went away on its own (owner stopped it).
                actions.push(format!("adopted engine pid={pid} ({image}) is gone — horn stands down, does not spawn"));
                self.state = HornState::NoServer;
                self.failures = 0;
                self.healthy_since = None;
                // Deliberately NO spawn: the engine's owner decides when it
                // returns; a horn that auto-spawns over an owner-managed port
                // would race the owner's own watchdog.
            }
            HornState::StrangerOnPort { pid, image } => {
                actions.push(format!("stranger pid={pid} ({image}) left the port"));
                self.state = HornState::NoServer;
            }
            HornState::Blocked { .. } => {
                actions.push("blocked: no spawn attempts until unblock()".into());
            }
            HornState::NoServer => {
                self.try_spawn(&mut actions);
            }
        }
        TickReport { state: self.state.clone(), actions }
    }

    /// Record a failure, park in Blocked when the budget is spent.
    fn enter_failure_state(&mut self, actions: &mut Vec<String>) {
        self.failures += 1;
        self.state = if self.failures >= self.settings.max_failures {
            actions.push(format!(
                "failures={} >= max: BLOCKED until unblock()",
                self.failures
            ));
            HornState::Blocked { failures: self.failures }
        } else {
            let wait = backoff_for(self.failures);
            self.next_spawn_at = Some(Instant::now() + wait);
            actions.push(format!("failures={}; backoff {:?}", self.failures, wait));
            HornState::NoServer
        };
        self.healthy_since = None;
    }

    fn try_spawn(&mut self, actions: &mut Vec<String>) {
        // Backoff / settle window: not yet.
        if let Some(at) = self.next_spawn_at {
            if Instant::now() < at {
                actions.push("settle/backoff window still open".into());
                return;
            }
        }
        // QQ2 doctrine: VRAM snapshot travels WITH the attempt (telemetry,
        // measured-first; see module doc for why it is not a free-VRAM gate).
        let vram = vram::probe();
        actions.push(format!(
            "spawn attempt: vram source={} dedicated_bytes={} (QQ2 snapshot)",
            vram.source,
            vram.total_dedicated_video_bytes()
        ));
        match self.world.spawn_engine(&self.settings) {
            Ok(pid) => {
                self.state = HornState::Spawned { pid };
                self.healthy_since = None; // health = port binds, next tick proves it
                actions.push(format!("engine spawned pid={pid}; waiting for port bind",));
            }
            Err(cause) => {
                actions.push(format!("spawn failed: {cause}"));
                self.enter_failure_state(actions);
            }
        }
    }

    /// Explicitly stop an engine WE spawned (adopted engines are refused).
    /// taskkill /T: the engine may have helpers. Returns once the port is
    /// free or the settle window expires — the caller then ticks normally.
    pub fn stop_own(&mut self) -> Result<(), String> {
        let pid = match &self.state {
            HornState::Spawned { pid } => *pid,
            HornState::Adopted { .. } => {
                return Err("refused: adopted engines are never killed by the horn (P5 law)".into());
            }
            s => return Err(format!("nothing to stop in state {s:?}")),
        };
        let _ = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/T", "/F"])
            .output();
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if !self.world.port_taken(&self.settings.engine_host, self.settings.engine_port) {
                self.state = HornState::NoServer;
                self.failures = 0;
                self.healthy_since = None;
                // Settle before any respawn (post-kill WDDM race law).
                self.next_spawn_at =
                    Some(Instant::now() + Duration::from_millis(self.settings.settle_ms));
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(250));
        }
        Err("port still bound after kill + wait".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// Scripted world for state-machine tests.
    struct FakeWorld {
        script: RefCell<VecDeque<WorldObs>>,
    }
    #[derive(Debug)]
    enum WorldObs {
        Taken(bool),
        Pid(Option<u32>),
        Image(Option<String>),
        Spawn(Result<u32, String>),
    }
    impl FakeWorld {
        fn new(obs: Vec<WorldObs>) -> Self {
            FakeWorld { script: RefCell::new(obs.into()) }
        }
    }
    impl EngineWorld for FakeWorld {
        fn port_taken(&mut self, _h: &str, _p: u16) -> bool {
            match self.script.borrow_mut().pop_front() {
                Some(WorldObs::Taken(b)) => b,
                other => panic!("unexpected probe; next={other:?}"),
            }
        }
        fn listening_pid(&mut self, _p: u16) -> Option<u32> {
            match self.script.borrow_mut().pop_front() {
                Some(WorldObs::Pid(p)) => p,
                other => panic!("unexpected pid resolve; next={other:?}"),
            }
        }
        fn image_name(&mut self, _p: u32) -> Option<String> {
            match self.script.borrow_mut().pop_front() {
                Some(WorldObs::Image(i)) => i,
                other => panic!("unexpected image resolve; next={other:?}"),
            }
        }
        fn spawn_engine(&mut self, _s: &HornSettings) -> Result<u32, String> {
            match self.script.borrow_mut().pop_front() {
                Some(WorldObs::Spawn(r)) => r,
                other => panic!("unexpected spawn; next={other:?}"),
            }
        }
    }

    fn small_settings() -> HornSettings {
        HornSettings {
            max_failures: 3,
            ..HornSettings::default()
        }
    }

    #[test]
    fn adopt_decision_requires_image_identity() {
        assert_eq!(
            adopt_decision(Some(42), Some("whisper-server.exe".into()), r"E:\x\whisper-server.exe"),
            Some((42, "whisper-server.exe".into()))
        );
        // Case-insensitive image, colon-anchored port is netstat's job; here:
        // a stranger image is never adopted.
        assert_eq!(
            adopt_decision(Some(7), Some("python.exe".into()), r"E:\x\whisper-server.exe"),
            None
        );
        assert_eq!(adopt_decision(None, None, "x"), None);
    }

    #[test]
    fn tick_adopts_known_image_and_never_kills_it() {
        let w = FakeWorld::new(vec![
            WorldObs::Taken(true),
            WorldObs::Pid(Some(313)),
            WorldObs::Image(Some("WHISPER-SERVER.EXE".into())),
        ]);
        let mut sup = Supervisor::new(small_settings(), w);
        let r = sup.tick();
        assert_eq!(
            r.state,
            HornState::Adopted { pid: 313, image: "WHISPER-SERVER.EXE".into() }
        );
        assert!(sup.stop_own().is_err()); // adopted: refused by law
    }

    #[test]
    fn tick_refuses_stranger_loudly() {
        let w = FakeWorld::new(vec![
            WorldObs::Taken(true),
            WorldObs::Pid(Some(9)),
            WorldObs::Image(Some("chrome.exe".into())),
        ]);
        let mut sup = Supervisor::new(small_settings(), w);
        let r = sup.tick();
        assert_eq!(r.state, HornState::StrangerOnPort { pid: 9, image: "chrome.exe".into() });
        assert!(r.actions.iter().any(|a| a.contains("NOT ours")));
    }

    #[test]
    fn adopted_engine_disappearing_means_stand_down_not_spawn() {
        // Tick 1: adopt. Tick 2: port free -> stand down (NO spawn call).
        let w = FakeWorld::new(vec![
            WorldObs::Taken(true),
            WorldObs::Pid(Some(5)),
            WorldObs::Image(Some("whisper-server.exe".into())),
            WorldObs::Taken(false),
        ]);
        let mut sup = Supervisor::new(small_settings(), w);
        let _ = sup.tick();
        let r = sup.tick();
        assert_eq!(r.state, HornState::NoServer);
        assert!(r.actions.iter().any(|a| a.contains("stands down")));
    }

    #[test]
    fn failures_back_off_then_block() {
        // Spawn fails 3x (max_failures=3) -> Blocked; unblock resets.
        // The backoff WINDOW is zeroed between ticks explicitly: its duration
        // is proven by backoff_table_is_exponential_capped; here we test the
        // state machine, not the wall clock.
        let w = FakeWorld::new(vec![
            WorldObs::Taken(false),
            WorldObs::Spawn(Err("missing".into())),
            WorldObs::Taken(false),
            WorldObs::Spawn(Err("missing".into())),
            WorldObs::Taken(false),
            WorldObs::Spawn(Err("missing".into())),
            WorldObs::Taken(false), // blocked: probe only, no spawn consumed
        ]);
        let mut sup = Supervisor::new(small_settings(), w);
        let r1 = sup.tick();
        assert_eq!(r1.state, HornState::NoServer); // failure #1, window set
        assert!(r1.actions.iter().any(|a| a.contains("backoff")));
        sup.next_spawn_at = None; // window elapsed (separately proven)
        let r2 = sup.tick();
        assert_eq!(r2.state, HornState::NoServer); // failure #2
        sup.next_spawn_at = None;
        let r3 = sup.tick();
        assert_eq!(r3.state, HornState::Blocked { failures: 3 });
        let r4 = sup.tick(); // stays blocked, no spawn attempt on the fake
        assert!(matches!(r4.state, HornState::Blocked { .. }));
        sup.unblock();
        assert_eq!(*sup.state(), HornState::NoServer);
    }

    #[test]
    fn backoff_table_is_exponential_capped() {
        assert_eq!(backoff_for(1), Duration::from_secs(2));
        assert_eq!(backoff_for(2), Duration::from_secs(4));
        assert_eq!(backoff_for(9), BACKOFF_CAP);
    }

    #[test]
    fn spawned_engine_binding_port_becomes_healthy_then_death_fails_once() {
        let w = FakeWorld::new(vec![
            WorldObs::Taken(false),
            WorldObs::Spawn(Ok(77)),
            WorldObs::Taken(true), // bound: healthy, ours
            WorldObs::Taken(false), // died
            WorldObs::Taken(false), // window open (NOT zeroed on purpose)
            WorldObs::Taken(false),
            WorldObs::Spawn(Ok(78)), // after zeroing the window: respawn
            WorldObs::Taken(true),
        ]);
        let mut sup = Supervisor::new(small_settings(), w);
        let r = sup.tick();
        assert_eq!(r.state, HornState::Spawned { pid: 77 });
        let r = sup.tick();
        assert_eq!(r.state, HornState::Spawned { pid: 77 }); // healthy, no action
        let r = sup.tick(); // death noticed: failure #1 recorded, window set
        assert_eq!(r.state, HornState::NoServer);
        assert!(r.actions.iter().any(|a| a.contains("died")));
        let r = sup.tick(); // window still open on the real clock -> settle line
        assert_eq!(r.state, HornState::NoServer);
        assert!(r.actions.iter().any(|a| a.contains("window still open")));
        sup.next_spawn_at = None; // window elapsed (separately proven)
        let r = sup.tick();
        assert_eq!(r.state, HornState::Spawned { pid: 78 });
        let r = sup.tick();
        assert_eq!(r.state, HornState::Spawned { pid: 78 }); // healthy again
    }

}
