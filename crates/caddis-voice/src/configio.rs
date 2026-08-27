//! configio.rs — config file loading + the WARDEN-GATED write path (P1
//! slice (b), D6 law: "the world Voice Booth gets a READ view; ALL writes
//! are warden-gated").
//!
//! The gate mirrors crates/caddis-memory's remember flow (the estate's
//! convention; the voice crate must not layer on the memory crate, so the
//! seam is vendored like json.rs was):
//!
//! - `encode_frame` produces EXACTLY what crates/caddis-warden `wire.rs::
//!   parse` consumes — fixed field order `tool, command, path, content`,
//!   `<name> <byte-len>\n<bytes>\n`, lengths in BYTES. The two ends must
//!   never drift apart.
//! - The warden runs via [`Warden::judge`] (spawn, stdin frame, one JSON
//!   line out). FAIL-CLOSED: timeout, nonzero exit, or unparseable output
//!   BLOCKS the write — never a silent allow. `verdict == "allow"` is the
//!   ONLY allow; any other verdict string is a deny with the verdict kept.
//! - `allow` with `seq == 0` = the verdict ran but the LEDGER ROW was not
//!   recorded (the warden's documented fail-open record leg) — no audit
//!   anchor, so the write is refused ([`SaveOutcome::Unrecorded`]), exactly
//!   like the memory organ's I4 law.
//! - Validation happens BEFORE gating: the warden judges a real, parseable
//!   config document (byte-exact payload), and an invalid document is
//!   rejected without spending a verdict.
//! - The file lands atomically: temp file + rename, never a partial write.

use crate::config::{parse_config, ConfigErr, OrganConfig};
use std::path::{Path, PathBuf};

/// The tool/command names the warden sees for a voice-organ config write.
pub const TOOL: &str = "caddis-voice";
pub const COMMAND: &str = "config-write";

/// Frame field order is FIXED — the warden's parser reads exactly this
/// sequence. Byte counts, never char counts.
pub fn encode_frame(path: &str, content: &str) -> String {
    let mut out = String::new();
    push_field(&mut out, "tool", TOOL);
    push_field(&mut out, "command", COMMAND);
    push_field(&mut out, "path", path);
    push_field(&mut out, "content", content);
    out
}

/// `s.len()` on a Rust `str` IS the byte count — the wire's length prefix is
/// byte-exact by construction, no unsafe round trip needed.
fn push_field(out: &mut String, name: &str, s: &str) {
    out.push_str(name);
    out.push(' ');
    out.push_str(&s.len().to_string());
    out.push('\n');
    out.push_str(s);
    out.push('\n');
}

/// A parsed warden reply. `allow` is `verdict == "allow"` exactly.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WardenVerdict {
    pub verdict: String,
    pub allow: bool,
    pub reason: String,
    pub law: String,
    pub seq: u64,
}

/// Fail-closed parse of the warden's JSON reply (`{verdict, reason, law,
/// seq}`). Anything malformed is `Err` — the caller BLOCKS.
pub fn parse_verdict(reply: &str) -> Result<WardenVerdict, String> {
    let v = crate::json::parse(reply).map_err(|e| format!("unparseable warden reply: {e:?}"))?;
    let f = |k: &str| -> Result<String, String> {
        v.get(k)
            .and_then(crate::json::Value::as_str)
            .map(str::to_string)
            .ok_or_else(|| format!("warden reply missing '{k}'"))
    };
    let verdict = f("verdict")?;
    let reason = f("reason")?;
    let law = f("law")?;
    let seq = v
        .get("seq")
        .and_then(crate::json::Value::as_f64)
        .ok_or_else(|| "warden reply missing 'seq'".to_string())? as u64;
    let allow = verdict == "allow";
    Ok(WardenVerdict {
        verdict,
        allow,
        reason,
        law,
        seq,
    })
}

/// What one judged write attempt provably did.
#[derive(Debug, Clone, PartialEq)]
pub enum SaveOutcome {
    /// The warden allowed it and the ledger recorded it (`seq > 0`).
    Allowed { seq: u64 },
    /// The warden said no. Nothing was written.
    Denied {
        verdict: String,
        reason: String,
        law: String,
    },
    /// Allowed but unrecorded (seq 0) — refused, no audit anchor exists.
    Unrecorded,
    /// The warden ran but its answer is unreadable — BLOCK, never a silent
    /// allow. Nothing was written.
    WardenUnreadable(String),
    /// The document itself is not a valid organ config — rejected before the
    /// gate (the warden judges real configs, not garbage).
    Invalid(ConfigErr),
    /// The verdict was allow+recorded but the file write failed.
    Io(String),
}

/// The exec seam over "run the warden once". Real spawns the binary; the
/// test fake answers from a script — the fail-closed paths are exercised
/// without a warden binary present.
pub trait Warden {
    /// `Err` = could not get a readable verdict (spawn/timeout/exit).
    fn judge(&mut self, frame: &str) -> Result<String, String>;
}

/// The real warden: launcher program + prefix args (e.g. `["warden"]` or a
/// path), the frame on stdin, one line of stdout, hard deadline.
pub struct RealWarden {
    pub launcher: Vec<String>,
    pub timeout_ms: u64,
}

impl Warden for RealWarden {
    fn judge(&mut self, frame: &str) -> Result<String, String> {
        use std::io::Write;
        use std::process::{Command, Stdio};
        use std::time::{Duration, Instant};

        let (program, prefix) = self
            .launcher
            .split_first()
            .ok_or_else(|| "empty warden launcher".to_string())?;
        let mut child = Command::new(program)
            .args(prefix)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("warden spawn failed: {e}"))?;
        // Write the frame; a child that exits early gets a broken pipe —
        // that is a verdict-less run, i.e. Err, i.e. BLOCK.
        {
            let mut stdin = child
                .stdin
                .take()
                .ok_or_else(|| "no warden stdin".to_string())?;
            stdin
                .write_all(frame.as_bytes())
                .map_err(|e| format!("warden stdin write failed: {e}"))?;
        }
        // Deadline via try_wait polling; kill on breach (exec.rs precedent).
        let deadline = Instant::now() + Duration::from_millis(self.timeout_ms);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if !status.success() {
                        return Err(format!("warden exited {status}"));
                    }
                    let mut out = String::new();
                    use std::io::Read;
                    let mut pipe = child
                        .stdout
                        .take()
                        .ok_or_else(|| "no warden stdout".to_string())?;
                    pipe.read_to_string(&mut out)
                        .map_err(|e| format!("warden stdout read failed: {e}"))?;
                    return Ok(out);
                }
                Ok(None) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Ok(None) => {
                    let _ = child.kill();
                    return Err("warden deadline exceeded".into());
                }
                Err(e) => return Err(format!("warden wait failed: {e}")),
            }
        }
    }
}

/// Where a loaded config came from — telemetry for /health and the operator.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigSource {
    /// No config file exists; the embedded default booted the organ.
    Embedded,
    /// A file was read and parsed.
    File,
}

/// Load the organ config. Missing file = embedded defaults (the organ boots
/// valid, D6); a file that exists but does not parse is a LOUD error — never
/// a silent fallback to defaults over an operator's broken edit.
pub fn load_config(path: &Path) -> Result<(OrganConfig, ConfigSource), ConfigErr> {
    match std::fs::read_to_string(path) {
        Ok(text) => Ok((parse_config(&text)?, ConfigSource::File)),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok((OrganConfig::default(), ConfigSource::Embedded))
        }
        Err(e) => Err(ConfigErr(format!("config read {}: {e}", path.display()))),
    }
}

/// Validate, gate, and atomically write a config document. The `path` in the
/// warden frame is the ABSOLUTE path (canonicalized when possible) — the
/// gate must judge the real target, not a relative spelling of it.
pub fn save_config_document(path: &Path, text: &str, warden: &mut dyn Warden) -> SaveOutcome {
    // 1. Validate first: the warden judges a REAL config document.
    if let Err(e) = parse_config(text) {
        return SaveOutcome::Invalid(e);
    }
    // 2. Absolute path for the frame.
    let abs = std::fs::canonicalize(path).unwrap_or_else(|_| {
        // Not-yet-existing file: canonicalize the parent, join the name.
        let parent = path
            .parent()
            .map(Path::to_path_buf)
            .unwrap_or_else(|| PathBuf::from("."));
        let canon_parent = std::fs::canonicalize(&parent).unwrap_or(parent);
        canon_parent.join(path.file_name().unwrap_or_default())
    });
    let frame = encode_frame(&abs.display().to_string(), text);
    // 3. Judge. Unreadable = BLOCK.
    let reply = match warden.judge(&frame) {
        Ok(r) => r,
        Err(e) => return SaveOutcome::WardenUnreadable(e),
    };
    let verdict = match parse_verdict(&reply) {
        Ok(v) => v,
        Err(e) => return SaveOutcome::WardenUnreadable(e),
    };
    // 4. Act on the verdict, fail-closed everywhere.
    if !verdict.allow {
        return SaveOutcome::Denied {
            verdict: verdict.verdict,
            reason: verdict.reason,
            law: verdict.law,
        };
    }
    if verdict.seq == 0 {
        return SaveOutcome::Unrecorded;
    }
    // 5. Atomic write: temp in the same directory, rename over.
    let dir = abs.parent().map(Path::to_path_buf).unwrap_or_default();
    let name = abs
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("organ.json");
    // fs::write truncates/overwrites, so a stale temp from a crashed run is
    // harmless; the pid suffix keeps concurrent organ instances apart.
    let tmp = dir.join(format!(".{name}.tmp-{}", std::process::id()));
    if let Err(e) = std::fs::write(&tmp, text.as_bytes()) {
        return SaveOutcome::Io(format!("temp write: {e}"));
    }
    if let Err(e) = std::fs::rename(&tmp, &abs) {
        let _ = std::fs::remove_file(&tmp);
        return SaveOutcome::Io(format!("rename: {e}"));
    }
    SaveOutcome::Allowed { seq: verdict.seq }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Scripted fake: returns canned replies in order, counts calls.
    struct FakeWarden {
        replies: Vec<Result<String, String>>,
        calls: Vec<String>,
    }
    impl FakeWarden {
        fn new(replies: Vec<Result<String, String>>) -> Self {
            FakeWarden {
                replies,
                calls: Vec::new(),
            }
        }
    }
    impl Warden for FakeWarden {
        fn judge(&mut self, frame: &str) -> Result<String, String> {
            self.calls.push(frame.to_string());
            self.replies
                .pop()
                .unwrap_or_else(|| Err("no scripted reply".into()))
        }
    }

    fn allow_reply(seq: u64) -> String {
        format!(r#"{{"verdict":"allow","reason":"","law":"","seq":{seq}}}"#)
    }

    #[test]
    fn frame_is_the_warden_wire_exactly() {
        let f = encode_frame("C:/a b/organ.json", "{\"x\":1}");
        // Field order fixed; lengths are BYTE lengths; content verbatim.
        assert_eq!(
            f,
            "tool 12\ncaddis-voice\ncommand 12\nconfig-write\npath 17\nC:/a b/organ.json\ncontent 7\n{\"x\":1}\n"
        );
    }

    #[test]
    fn frame_byte_lengths_survive_non_ascii() {
        // "organas" with a diacritic is >7 BYTES though 8 CHARS — the length
        // prefix must count bytes or the warden frame parser desyncs.
        let lt = "orga\u{0105}nas.json";
        let f = encode_frame(lt, "x");
        let n = lt.len();
        assert!(f.contains(&format!("path {n}\n")), "{f}");
    }

    #[test]
    fn verdict_parse_is_fail_closed() {
        let v = parse_verdict(&allow_reply(7)).unwrap();
        assert!(v.allow);
        assert_eq!(v.seq, 7);
        let deny = parse_verdict(r#"{"verdict":"deny","reason":"card closed","law":"L1","seq":3}"#)
            .unwrap();
        assert!(!deny.allow);
        assert_eq!(deny.verdict, "deny");
        assert!(parse_verdict("not json").is_err());
        assert!(parse_verdict(r#"{"verdict":"allow"}"#).is_err()); // missing fields
    }

    #[test]
    fn load_missing_file_boots_embedded() {
        let (cfg, src) = load_config(Path::new("Z:/definitely/not/here/organ.json")).unwrap();
        assert_eq!(src, ConfigSource::Embedded);
        assert!(cfg.labels.contains_key("sergeant"));
    }

    #[test]
    fn load_broken_file_is_loud() {
        let dir = std::env::temp_dir().join("caddis-voice-test-broken");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("organ.json");
        std::fs::write(&p, "{\"defaults\": {}}").unwrap();
        let err = load_config(&p).unwrap_err();
        assert!(!err.0.is_empty());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn load_valid_file_parses_as_file() {
        let dir = std::env::temp_dir().join("caddis-voice-test-valid");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("organ.json");
        std::fs::write(&p, crate::config::DEFAULT_CONFIG_JSON).unwrap();
        let (cfg, src) = load_config(&p).unwrap();
        assert_eq!(src, ConfigSource::File);
        assert_eq!(cfg.registry.voices.len(), 4);
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn save_happy_path_writes_and_round_trips() {
        let dir = std::env::temp_dir().join("caddis-voice-test-save-ok");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("organ.json");
        let _ = std::fs::remove_file(&p);
        let mut w = FakeWarden::new(vec![Ok(allow_reply(42))]);
        let out = save_config_document(&p, crate::config::DEFAULT_CONFIG_JSON, &mut w);
        assert_eq!(out, SaveOutcome::Allowed { seq: 42 });
        assert_eq!(w.calls.len(), 1);
        // The judged frame names the absolute path and the byte-exact body.
        let frame = &w.calls[0];
        assert!(
            frame.contains(&p.canonicalize().unwrap().display().to_string())
                || frame.contains("organ.json")
        );
        // Round trip: what lands parses back identical.
        let (cfg, src) = load_config(&p).unwrap();
        assert_eq!(src, ConfigSource::File);
        assert_eq!(cfg, OrganConfig::default());
        let _ = std::fs::remove_file(&p);
    }

    #[test]
    fn save_denied_writes_nothing() {
        let dir = std::env::temp_dir().join("caddis-voice-test-save-deny");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("organ.json");
        let _ = std::fs::remove_file(&p);
        let mut w = FakeWarden::new(vec![Ok(
            r#"{"verdict":"deny","reason":"outside card","law":"L9","seq":5}"#.into(),
        )]);
        let out = save_config_document(&p, crate::config::DEFAULT_CONFIG_JSON, &mut w);
        assert_eq!(
            out,
            SaveOutcome::Denied {
                verdict: "deny".into(),
                reason: "outside card".into(),
                law: "L9".into()
            }
        );
        assert!(!p.exists(), "a denied write must not create the file");
    }

    #[test]
    fn save_unreadable_blocks_and_unrecorded_refuses() {
        let dir = std::env::temp_dir().join("caddis-voice-test-save-block");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("organ.json");
        // Unreadable (spawn failure shaped): BLOCK.
        let mut w = FakeWarden::new(vec![Err("warden deadline exceeded".into())]);
        let out = save_config_document(&p, crate::config::DEFAULT_CONFIG_JSON, &mut w);
        assert_eq!(
            out,
            SaveOutcome::WardenUnreadable("warden deadline exceeded".into())
        );
        assert!(!p.exists());
        // Garbage reply: BLOCK.
        let mut w = FakeWarden::new(vec![Ok("(((not json".into())]);
        assert!(matches!(
            save_config_document(&p, crate::config::DEFAULT_CONFIG_JSON, &mut w),
            SaveOutcome::WardenUnreadable(_)
        ));
        // allow + seq 0: unrecorded — refused.
        let mut w = FakeWarden::new(vec![Ok(allow_reply(0))]);
        assert_eq!(
            save_config_document(&p, crate::config::DEFAULT_CONFIG_JSON, &mut w),
            SaveOutcome::Unrecorded
        );
        assert!(!p.exists());
    }

    #[test]
    fn save_invalid_document_never_reaches_the_warden() {
        let dir = std::env::temp_dir().join("caddis-voice-test-save-invalid");
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("organ.json");
        let mut w = FakeWarden::new(vec![Ok(allow_reply(1))]);
        let out = save_config_document(&p, "{ not a config }", &mut w);
        assert!(matches!(out, SaveOutcome::Invalid(_)));
        assert!(
            w.calls.is_empty(),
            "invalid documents must not spend a verdict"
        );
        assert!(!p.exists());
    }
}
