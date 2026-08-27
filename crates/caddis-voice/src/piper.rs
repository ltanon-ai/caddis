//! piper.rs — the OFFLINE EN adapter (R-E internal set, lane Offline).
//!
//! Drives the proven `piper.exe` CLI exactly the way the daemon always
//! has (peluda_voice/tts.py, the invariant source): text → temp file in,
//! WAV → temp file out, `--length-scale` prosody, killable child under a
//! [`ChildScope`] job (QQ2/F2: a wedged render dies, the organ lives).
//!
//! Budget law: the kill deadline is the daemon's PROVEN FLAT 20 s —
//! startup-dominated cost with ~1.6x jitter; the old 8 s ceiling blew 208
//! times under fleet load and lost each narration. The registry's
//! `render_cap_ms` is F-A4 TELEMETRY (over-cap is reported, never used to
//! kill). The horn's `engine_args_override` precedent applies: the argv
//! override is a TEST/STUB lane only; production argv is the daemon-proven
//! template.

use crate::adapter::{sanitize_text, validate_wav, AdapterErr, AudioFormat, RenderedAudio};
use crate::job::ChildScope;
use crate::registry::VoiceSpec;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// The ported daemon budget (PIPER_TIMEOUT_S = 20.0): startup-dominated,
/// flat on purpose, measured 1.6-4.6 s idle with ~1.6x jitter.
pub const PIPER_KILL_DEADLINE_MS: u128 = 20_000;

/// Poll cadence while waiting for piper to exit (25 ms; the cost is a
/// try_wait syscall, not a context switch storm).
const POLL_MS: u64 = 25;

/// CREATE_NO_WINDOW — a render must never flash a console (the hidden-jobs
/// law applies to every spawned organ child).
#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Debug, Clone, PartialEq)]
pub struct PiperPaths {
    pub exe: String,
    pub model: String,
    /// Voice model config; None = `<model>.json` (piper's own convention).
    pub model_config: Option<String>,
}

static SEQ: AtomicU64 = AtomicU64::new(0);

/// Unique temp-file pair for one render (parallel-test lesson: shared
/// names eat each other; nanos + per-process counter + pid).
fn temp_pair() -> (PathBuf, PathBuf) {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let seq = SEQ.fetch_add(1, Ordering::Relaxed);
    let pid = std::process::id();
    let base = std::env::temp_dir().join(format!("caddis-piper-{pid}-{nanos}-{seq}"));
    (base.with_extension("txt"), base.with_extension("wav"))
}

#[derive(Debug, Clone)]
pub struct PiperAdapter {
    paths: PiperPaths,
    /// F-A4 declared render cap — telemetry only.
    cap_ms: u32,
    /// Kill timer; the proven 20 s unless a test shrinks it.
    kill_deadline_ms: u128,
    /// TEST/STUB lane only (horn precedent). `{IN}` / `{OUT}` are
    /// substituted with the per-render temp paths. Production renders
    /// never set this.
    argv_override: Option<Vec<String>>,
}

impl PiperAdapter {
    pub fn new(paths: PiperPaths, cap_ms: u32) -> Self {
        PiperAdapter { paths, cap_ms, kill_deadline_ms: PIPER_KILL_DEADLINE_MS, argv_override: None }
    }

    /// TEST LANE ONLY: full argv replacement with `{IN}`/`{OUT}`
    /// placeholders, plus a shrunken kill deadline.
    pub fn with_stub_argv(mut self, argv: Vec<String>, kill_deadline_ms: u128) -> Self {
        self.argv_override = Some(argv);
        self.kill_deadline_ms = kill_deadline_ms;
        self
    }

    fn argv(&self, in_path: &str, out_path: &str, length_scale: f64) -> Vec<String> {
        if let Some(ov) = &self.argv_override {
            return ov
                .iter()
                .map(|a| a.replace("{IN}", in_path).replace("{OUT}", out_path))
                .collect();
        }
        let cfg = self
            .paths
            .model_config
            .clone()
            .unwrap_or_else(|| format!("{}.json", self.paths.model));
        vec![
            self.paths.exe.clone(),
            "-m".into(),
            self.paths.model.clone(),
            "-c".into(),
            cfg,
            "-i".into(),
            in_path.into(),
            "-f".into(),
            out_path.into(),
            "--length-scale".into(),
            format!("{length_scale}"),
        ]
    }

    /// Render one sanitized text to WAV bytes. The GA3 breaker is enforced
    /// by the caller (the dispatch layer owns the breaker so both lanes
    /// share one truth).
    pub fn render(
        &self,
        voice: &VoiceSpec,
        text: &str,
        length_scale: f64,
    ) -> Result<RenderedAudio, AdapterErr> {
        let s = sanitize_text(text)?;
        if voice.generator != "piper" {
            return Err(AdapterErr(format!("piper: voice {} is not a piper voice", voice.id)));
        }
        if self.paths.exe.is_empty() || self.paths.model.is_empty() {
            return Err(AdapterErr("piper: exe/model not configured".into()));
        }

        let (in_path, out_path) = temp_pair();
        if let Err(e) = std::fs::write(&in_path, &s.text) {
            return Err(AdapterErr(format!("piper: write input failed: {e}")));
        }

        let started = Instant::now();
        let argv = self.argv(&in_path.to_string_lossy(), &out_path.to_string_lossy(), length_scale);
        let mut cmd = Command::new(&argv[0]);
        cmd.args(&argv[1..]);
        // Both streams null (the daemon captured them for the same
        // reason): the render's contract is the OUT file, not chatter,
        // and organ/test output stays clean.
        cmd.stdout(std::process::Stdio::null());
        cmd.stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            cmd.creation_flags(CREATE_NO_WINDOW);
        }
        let child = cmd.spawn().map_err(|e| {
            let _ = std::fs::remove_file(&in_path);
            AdapterErr(format!("piper: spawn failed: {e}"))
        })?;
        let pid = child.id();

        // QQ2/F2: the job kills this child if we die mid-render; on the
        // timeout path, dropping the scope is the kill.
        let scope = ChildScope::create().map_err(|e| AdapterErr(format!("piper: job scope: {e}")));
        if let Ok(s) = &scope {
            if let Err(e) = s.assign_pid(pid) {
                return Err(AdapterErr(format!("piper: job assign: {e}")));
            }
        }

        let mut child = child;
        let st = loop {
            match child.try_wait() {
                Ok(Some(st)) => break st,
                Ok(None) => {}
                Err(e) => {
                    drop(scope);
                    let _ = std::fs::remove_file(&in_path);
                    return Err(AdapterErr(format!("piper: wait failed: {e}")));
                }
            }
            if started.elapsed().as_millis() > self.kill_deadline_ms {
                // Drop = kernel kill of the render child.
                drop(scope);
                let _ = child.kill();
                let _ = std::fs::remove_file(&in_path);
                let _ = std::fs::remove_file(&out_path);
                return Err(AdapterErr(format!(
                    "piper: render exceeded {}ms kill deadline",
                    self.kill_deadline_ms
                )));
            }
            std::thread::sleep(std::time::Duration::from_millis(POLL_MS));
        };
        drop(scope);
        let _ = std::fs::remove_file(&in_path);

        if !st.success() {
            let _ = std::fs::remove_file(&out_path);
            return Err(AdapterErr(format!("piper: exited {st}")));
        }
        let bytes = match std::fs::read(&out_path) {
            Ok(b) => b,
            Err(e) => return Err(AdapterErr(format!("piper: no output wav: {e}"))),
        };
        let _ = std::fs::remove_file(&out_path);
        validate_wav(&bytes)?;

        let elapsed_ms = started.elapsed().as_millis();
        Ok(RenderedAudio {
            bytes,
            format: AudioFormat::Wav,
            generator: "piper".into(),
            voice: voice.id.clone(),
            elapsed_ms,
            cap_ms: self.cap_ms,
            over_cap: elapsed_ms > u128::from(self.cap_ms),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::lang::Lang;

    fn voice() -> VoiceSpec {
        VoiceSpec { id: "en_US-amy".into(), generator: "piper".into(), lang: Lang::En }
    }

    /// Secret-shaped text fixture built at runtime (warden law: no
    /// key-shaped literal in source, even in tests).
    fn secret_text() -> String {
        let mut s = String::new();
        s.push('s');
        s.push('k');
        s.push('-');
        for _ in 0..32 {
            s.push('X');
        }
        s
    }

    /// Per-test canned WAV (parallel-suite law: shared temp files eat
    /// each other — unique tag per test).
    fn canned_wav(tag: &str) -> PathBuf {
        // A real, tiny RIFF/WAVE (0.1s of silence, 8kHz mono 16-bit).
        let sr = 8000usize;
        let data: Vec<u8> = vec![0u8; sr / 10 * 2];
        let mut wav = Vec::new();
        wav.extend_from_slice(b"RIFF");
        wav.extend_from_slice(&(36u32 + data.len() as u32).to_le_bytes());
        wav.extend_from_slice(b"WAVEfmt ");
        wav.extend_from_slice(&16u32.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&1u16.to_le_bytes());
        wav.extend_from_slice(&(sr as u32).to_le_bytes());
        wav.extend_from_slice(&((sr * 2) as u32).to_le_bytes());
        wav.extend_from_slice(&2u16.to_le_bytes());
        wav.extend_from_slice(&16u16.to_le_bytes());
        wav.extend_from_slice(b"data");
        wav.extend_from_slice(&(data.len() as u32).to_le_bytes());
        wav.extend_from_slice(&data);
        let p = std::env::temp_dir().join(format!("caddis-piper-test-canned-{tag}.wav"));
        std::fs::write(&p, &wav).unwrap();
        p
    }

    fn stub_adapter(tag: &str) -> (PiperAdapter, PathBuf) {
        let canned = canned_wav(tag);
        let a = PiperAdapter::new(
            PiperPaths { exe: "cmd".into(), model: "unused".into(), model_config: None },
            1500,
        )
        .with_stub_argv(
            vec![
                "cmd".into(),
                "/c".into(),
                "copy".into(),
                "/Y".into(),
                canned.to_string_lossy().into_owned(),
                "{OUT}".into(),
            ],
            8_000,
        );
        (a, canned)
    }

    #[test]
    fn stub_lane_renders_valid_wav() {
        let (a, canned) = stub_adapter("render");
        let r = a.render(&voice(), "Labas, this is a test.", 1.0).unwrap();
        assert_eq!(r.format, AudioFormat::Wav);
        assert_eq!(r.generator, "piper");
        assert!(!r.bytes.is_empty());
        validate_wav(&r.bytes).unwrap();
        let _ = std::fs::remove_file(canned);
    }

    #[test]
    fn stub_lane_timeout_kills_child() {
        // A stub that sleeps ~30s: the kill deadline must fire and the
        // child die under it.
        let a = PiperAdapter::new(
            PiperPaths { exe: "cmd".into(), model: "unused".into(), model_config: None },
            1500,
        )
        .with_stub_argv(
            // `ping -n 30` sleeps without stdin; `pause` would exit at
            // once on the null stdio render children get.
            vec!["cmd".into(), "/c".into(), "ping".into(), "-n".into(), "30".into(), "127.0.0.1".into()],
            300,
        );
        let started = Instant::now();
        let e = a.render(&voice(), "hang test", 1.0).unwrap_err();
        assert!(e.0.contains("kill deadline"), "unexpected err: {e}");
        // Generous for parallel-suite load; the invariant is "not the
        // stub's full 30s".
        assert!(started.elapsed().as_millis() < 15_000);
        // A follow-up render works — no leaked lock/state after a kill.
        let (ok, canned) = stub_adapter("afterkill");
        assert!(ok.render(&voice(), "after kill", 1.0).is_ok());
        let _ = std::fs::remove_file(canned);
    }

    #[test]
    fn non_wav_output_rejected() {
        let junk = std::env::temp_dir().join("caddis-piper-test-junk.bin");
        std::fs::write(&junk, b"this is definitely not a wav").unwrap();
        let a = PiperAdapter::new(
            PiperPaths { exe: "cmd".into(), model: "unused".into(), model_config: None },
            1500,
        )
        .with_stub_argv(
            vec![
                "cmd".into(),
                "/c".into(),
                "copy".into(),
                "/Y".into(),
                junk.to_string_lossy().into_owned(),
                "{OUT}".into(),
            ],
            8_000,
        );
        let e = a.render(&voice(), "junk out", 1.0).unwrap_err();
        assert!(e.0.contains("GA2") || e.0.contains("RIFF"), "unexpected err: {e}");
        let _ = std::fs::remove_file(junk);
    }

    #[test]
    fn guard_order_text_and_voice_first() {
        let (a, canned) = stub_adapter("guard");
        // Secret-shaped text is refused before any process spawns.
        let e = a.render(&voice(), &secret_text(), 1.0).unwrap_err();
        assert!(e.to_string().starts_with("adapter: text"), "unexpected err: {e}");
        // Wrong-generator voice refused.
        let v2 = VoiceSpec { id: "lt-LT-OnaNeural".into(), generator: "ona".into(), lang: Lang::Lt };
        let e2 = a.render(&v2, "ok text", 1.0).unwrap_err();
        assert!(e2.0.contains("not a piper voice"), "unexpected err: {e2}");
        let _ = std::fs::remove_file(canned);
    }

    #[test]
    fn production_argv_is_daemon_template() {
        let a = PiperAdapter::new(
            PiperPaths { exe: "C:\\piper.exe".into(), model: "C:\\amy.onnx".into(), model_config: None },
            1500,
        );
        let argv = a.argv("IN.txt", "OUT.wav", 1.1);
        assert_eq!(
            argv,
            vec![
                "C:\\piper.exe".to_string(),
                "-m".into(),
                "C:\\amy.onnx".into(),
                "-c".into(),
                "C:\\amy.onnx.json".into(),
                "-i".into(),
                "IN.txt".into(),
                "-f".into(),
                "OUT.wav".into(),
                "--length-scale".into(),
                "1.1".into(),
            ]
        );
        assert_eq!(a.kill_deadline_ms, PIPER_KILL_DEADLINE_MS);
    }
}
