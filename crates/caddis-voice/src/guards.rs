//! guards.rs — the /transcribe request guards (P2 Horn: token + Host + size).
//!
//! Ported from `stt-daemon/stt_http.py`, where each guard exists because a
//! measured attack or accident earned it:
//!
//! - **Host guard (DNS rebinding)** — any web page the operator visits can
//!   fetch `http://127.0.0.1:8785/...`; a rebinding DNS name would make that
//!   request same-origin to the attacker. So the Host header must be exactly
//!   `127.0.0.1:<port>` or `localhost:<port>` (raw socket clients may omit it).
//!   Anything else is 421, before any body byte is read.
//! - **Token guard** — `X-STT-Token` compared against the token FILE, read
//!   fresh per request (rotation is a file write, not a restart). Constant
//!   time, because a timing oracle on a localhost token check is still an
//!   oracle. The token VALUE is never logged, never serialized, never echoed.
//! - **Size policy** — Content-Length REQUIRED and bounded (64 MiB), chunked
//!   REFUSED: a hand-rolled server must never turn an unbounded body into an
//!   unbounded read. 411/413 carry the rejection.

use std::fs;
use std::path::{Path, PathBuf};

/// The upload cap, byte-identical intent to the daemon's 64 MiB.
pub const MAX_UPLOAD_BYTES: usize = 64 * 1024 * 1024;

/// One guard verdict, ready to become an HTTP status without further thought.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GuardVerdict {
    /// All guards relevant to this stage passed.
    Pass,
    /// 421 — Host is not a loopback literal for our port (DNS-rebinding shape).
    BadHost,
    /// 401 — token missing or wrong.
    Unauthorized,
    /// 411 — no usable Content-Length (including chunked refusal).
    LengthRequired,
    /// 413 — declared body over the upload cap.
    TooLarge,
}

impl GuardVerdict {
    pub fn status(&self) -> u16 {
        match self {
            GuardVerdict::Pass => 200,
            GuardVerdict::BadHost => 421,
            GuardVerdict::Unauthorized => 401,
            GuardVerdict::LengthRequired => 411,
            GuardVerdict::TooLarge => 413,
        }
    }

    pub fn error_body(&self) -> String {
        let msg = match self {
            GuardVerdict::Pass => "ok",
            GuardVerdict::BadHost => "bad host",
            GuardVerdict::Unauthorized => "unauthorized",
            GuardVerdict::LengthRequired => "content-length required",
            GuardVerdict::TooLarge => "file too large",
        };
        format!("{{\"error\":\"{msg}\"}}")
    }
}

/// Case-insensitive header lookup over a `Name: value` list. Returns the
/// trimmed value of the FIRST match (header names are case-insensitive; the
/// organ speaks to real browsers, not to spec-ideal clients).
pub fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.trim())
}

/// Host must be `127.0.0.1:port` or `localhost:port`; absent is allowed for
/// raw socket clients (the daemon precedent). Everything else: rebinding.
pub fn host_ok(headers: &[(String, String)], port: u16) -> bool {
    match header(headers, "Host") {
        None => true,
        Some(h) => h == format!("127.0.0.1:{port}") || h == format!("localhost:{port}"),
    }
}

/// Constant-time equality for same-length secrets; length differences leak
/// only the length, which the attacker already knows from the file's size
/// class. (A localhost timing oracle is still an oracle.)
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (x, y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

/// The token source: a file path, read fresh per request. Construct from the
/// config; the daemon's live file is the parallel-run default so existing
/// clients (opener, mic.html) keep working against the organ unchanged.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenGuard {
    pub path: PathBuf,
}

impl TokenGuard {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        TokenGuard { path: path.into() }
    }

    /// Read + trim the token file. `Ok(None)` = unreadable file — treated by
    /// the caller as FAIL-CLOSED (no token authority = nobody passes).
    pub fn current(&self) -> std::io::Result<Option<String>> {
        match fs::read_to_string(&self.path) {
            Ok(s) => Ok(Some(s.trim().to_string())),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// Validate `X-STT-Token` against the file. Empty/missing file denies
    /// everyone (fail-closed); the token value never leaves this function.
    pub fn check(&self, headers: &[(String, String)]) -> bool {
        let presented = header(headers, "X-STT-Token").unwrap_or("");
        let expected = match self.current() {
            Ok(Some(t)) => t,
            _ => return false,
        };
        if presented.is_empty() || expected.is_empty() {
            return false;
        }
        constant_time_eq(presented.as_bytes(), expected.as_bytes())
    }
}

/// The full pre-body gate for POST /transcribe, in the daemon's order:
/// host → (token is checked by the caller before body read) → chunked →
/// length present → length bounded. Kept as one function so the order is
/// auditable in one place.
pub fn body_policy(headers: &[(String, String)]) -> GuardVerdict {
    if header(headers, "Transfer-Encoding")
        .map(|v| v.to_ascii_lowercase().contains("chunked"))
        .unwrap_or(false)
    {
        return GuardVerdict::LengthRequired;
    }
    let raw = header(headers, "Content-Length").unwrap_or("");
    let len: usize = match raw.parse() {
        Ok(n) => n,
        Err(_) => return GuardVerdict::LengthRequired,
    };
    if len == 0 {
        return GuardVerdict::LengthRequired;
    }
    if len > MAX_UPLOAD_BYTES {
        return GuardVerdict::TooLarge;
    }
    GuardVerdict::Pass
}

/// True if `candidate` (already resolved or not) stays under one of the
/// allowed roots. Symlinks are resolved first — an allowlist that a junction
/// steps over is not an allowlist. (The 'path' field source, ported from the
/// daemon's path_under_allowed_root.)
pub fn path_under_allowed_root(candidate: &Path, roots: &[PathBuf]) -> bool {
    let Ok(resolved) = candidate.canonicalize() else {
        return false;
    };
    roots.iter().any(|root| {
        match root.canonicalize() {
            Ok(r) => resolved.starts_with(&r),
            Err(_) => false,
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hdrs(pairs: &[(&str, &str)]) -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    }

    #[test]
    fn host_guard_accepts_only_loopback_literals_for_our_port() {
        let ok = hdrs(&[("Host", "127.0.0.1:8785"), ("X", "y")]);
        assert!(host_ok(&ok, 8785));
        let ok2 = hdrs(&[("host", "localhost:8785")]); // case-insensitive name
        assert!(host_ok(&ok2, 8785));
        let none = hdrs(&[("Something", "else")]);
        assert!(host_ok(&none, 8785)); // raw socket clients

        let evil = hdrs(&[("Host", "evil.example:8785")]);
        assert!(!host_ok(&evil, 8785));
        let other_port = hdrs(&[("Host", "127.0.0.1:8765")]);
        assert!(!host_ok(&other_port, 8785));
        let lan = hdrs(&[("Host", "100.122.146.70:8785")]);
        assert!(!host_ok(&lan, 8785));
    }

    #[test]
    fn body_policy_refuses_chunked_zero_and_oversize() {
        assert_eq!(
            body_policy(&hdrs(&[("Transfer-Encoding", "chunked"), ("Content-Length", "5")])),
            GuardVerdict::LengthRequired
        );
        assert_eq!(
            body_policy(&hdrs(&[("Content-Length", "0")])),
            GuardVerdict::LengthRequired
        );
        assert_eq!(
            body_policy(&hdrs(&[("Content-Length", (MAX_UPLOAD_BYTES + 1).to_string().as_str())])),
            GuardVerdict::TooLarge
        );
        assert_eq!(
            body_policy(&hdrs(&[("Content-Length", "not-a-number")])),
            GuardVerdict::LengthRequired
        );
        assert_eq!(
            body_policy(&hdrs(&[("Content-Length", "1024")])),
            GuardVerdict::Pass
        );
    }

    #[test]
    fn token_check_is_exact_and_fail_closed() {
        let dir = std::env::temp_dir().join(format!("caddis-voice-guard-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let tf = dir.join("token.txt");
        fs::write(&tf, "secret-one\n").unwrap();
        let guard = TokenGuard::new(&tf);

        assert!(guard.check(&hdrs(&[("X-STT-Token", "secret-one")])));
        assert!(!guard.check(&hdrs(&[("X-STT-Token", "secret-two")])));
        assert!(!guard.check(&hdrs(&[("X-STT-Token", "")]))); // empty never matches
        assert!(!guard.check(&[])); // missing header

        // rotation is a file write, not a restart
        fs::write(&tf, "secret-two").unwrap();
        assert!(guard.check(&hdrs(&[("X-STT-Token", "secret-two")])));
        assert!(!guard.check(&hdrs(&[("X-STT-Token", "secret-one")])));

        // missing file: deny everyone, no panic
        let missing = TokenGuard::new(dir.join("nope.txt"));
        assert!(!missing.check(&hdrs(&[("X-STT-Token", "anything")])));
        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn path_allowlist_resolves_and_refuses() {
        let dir = std::env::temp_dir().join(format!("caddis-voice-paths-{}", std::process::id()));
        let root = dir.join("allowed");
        let outside = dir.join("outside");
        fs::create_dir_all(&root).unwrap();
        fs::create_dir_all(&outside).unwrap();
        let inside_file = root.join("a.wav");
        fs::write(&inside_file, b"x").unwrap();
        let outside_file = outside.join("b.wav");
        fs::write(&outside_file, b"x").unwrap();

        assert!(path_under_allowed_root(&inside_file, std::slice::from_ref(&root)));
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub").join("c.wav"), b"x").unwrap();
        assert!(path_under_allowed_root(
            &root.join("sub").join("c.wav"),
            std::slice::from_ref(&root)
        ));
        assert!(!path_under_allowed_root(&outside_file, std::slice::from_ref(&root)));
        assert!(!path_under_allowed_root(&PathBuf::from("does-not-exist.wav"), std::slice::from_ref(&root)));
        fs::remove_dir_all(&dir).ok();
    }
}
