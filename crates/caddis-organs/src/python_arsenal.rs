//! python_arsenal.rs — CARD-0250. The Python arsenal organ.
//!
//! The operator's insight: omp and Claude let models write Python
//! in-session and use it as a tool. In caddis this becomes an ORGAN:
//! the model writes a function, the warden verifies it's safe, it's
//! stored in the lineage's arsenal, and FUTURE sessions use it as a
//! command — paying 50 tokens instead of 5000 for the same insight.
//!
//! The SAFETY GATE (`is_safe`) is the critical piece: it DENIES code
//! that can escape its sandbox and ALLOWs pure computational readers.
//! The warden pattern (deny/steer/allow) applies BEFORE first run.
//!
//! Storage: `~/.caddis/arsenal/<lineage>/<name>.py` — plain files,
//! versioned by the existing git home. [`invoke`] runs a stored entry
//! via a restricted Python subprocess; output capped at 10KB.
//!
//! Composting: entries with `uses == 0` across [`STAGNANT_WINDOW`]
//! epochs are candidates for removal (the déjà-vu organ measures
//! this; the host disposes).
//!
//! Zero runtime deps beyond caddis-core; sync, std only.

use std::fs;
use std::path::Path;
use std::process::Command;

/// Re-export the estate's stagnant-window constant — one constant,
/// never a second (CARD-0244 attention, CARD-0246 kv_bridge).
pub use crate::eddy_law::STAGNANT_WINDOW;

/// The safety verdict — mirrors the warden's deny/steer/allow. The
/// arsenal gate has no steer: the code is either safe to run or it
/// is not. Deny is fail-closed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SafetyVerdict {
    Allow,
    Deny,
}

/// One stored tool: the model wrote it, the warden verified it, the
/// lineage reuses it. `uses` counts invocations — the composting
/// signal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ArsenalEntry {
    pub name: String,
    pub code: String,
    pub created_by_model: String,
    pub uses: u64,
}

/// The maximum output [`invoke`] returns — 10KB. A runaway print
/// must not blow the context budget.
const OUTPUT_CAP: usize = 10_000;

/// Patterns whose presence in code is an unconditional DENY. Each is
/// a substring checked against the raw source — the gate is
/// conservative: if the token appears, the code is denied, regardless
/// of context. False positives (a string literal containing "eval")
/// are acceptable: the cost of a missed escape is higher than the
/// cost of rejecting a safe tool.
const DENY_TOKENS: &[&str] = &[
    "eval(",
    "exec(",
    "compile(",
    "subprocess",
    "os.system",
    "os.popen",
    "socket",
    "requests",
    "urllib",
    "http.client",
    "httplib",
    "__import__",
    "os.environ",
    "os.exec",
    "os.spawn",
    "pty",
    "ctypes",
    "cffi",
    "pickle",
    "marshal",
    "shutil",
    "tempfile",
    "sys.exit",
    "signal",
];

/// Write-mode flags that DENY an `open(...)` call. A read-only
/// `open('f', 'r')` or bare `open('f')` is ALLOWED; any write/append
/// mode is DENIED — writes outside the arsenal dir are forbidden.
const WRITE_MODES: &[&str] = &[
    "'w'", "'a'", "'wb'", "'ab'", "'w+'", "'a+'", "'r+'", "'x'", "\"w\"", "\"a\"", "\"wb\"",
    "\"ab\"", "\"w+\"", "\"a+\"", "\"r+\"", "\"x\"",
];

/// The safety gate: is this code safe to run in the arsenal sandbox?
///
/// DENY on: `eval`/`exec`/`compile` (dynamic code), `subprocess`/
/// `os.system` (process spawn), `__import__` with variable name
/// (dynamic import), `open(...,'w'|'a')` outside the arsenal dir
/// (writes), `socket`/`requests`/`http` (network), `os.environ`
/// (secret reads).
///
/// ALLOW on: read-only file I/O, string/list/dict operations, math,
/// json/csv parsing, `re`, `pathlib.Path.read_*`.
///
/// Conservative: a deny-token match in a comment or string literal
/// still denies. The organ's purpose is safe reuse, not syntactic
/// precision — rejecting a safe tool costs 50 tokens; missing an
/// escape vector costs the lineage. Write modes are checked only
/// near `open(` calls so JSON keys like `{"a":1}` do not false-trip.
pub fn is_safe(code: &str) -> SafetyVerdict {
    for tok in DENY_TOKENS {
        if code.contains(tok) {
            return SafetyVerdict::Deny;
        }
    }
    if has_write_open(code) {
        return SafetyVerdict::Deny;
    }
    SafetyVerdict::Allow
}

/// Scan for `open(...)` calls with a write-mode flag. We find each
/// `open(` occurrence and check whether a write-mode token appears
/// within the next 80 chars (enough for `open(path, 'w')`). This
/// avoids false-tripping on JSON keys or string literals containing
/// single letters.
fn has_write_open(code: &str) -> bool {
    let bytes = code.as_bytes();
    let mut i = 0;
    while i + 5 <= bytes.len() {
        if &bytes[i..i + 5] == b"open(" {
            let end = (i + 80).min(bytes.len());
            let window = &code[i + 5..end];
            for mode in WRITE_MODES {
                if window.contains(mode) {
                    return true;
                }
            }
        }
        i += 1;
    }
    false
}

/// Read an arsenal entry from `<arsenal_dir>/<name>.py`. Returns
/// the file contents as the `code` field, `uses` 0 (the host
/// tracks invocations via the existing observe nerve).
pub fn read_entry(arsenal_dir: &Path, name: &str) -> Option<ArsenalEntry> {
    let path = arsenal_dir.join(format!("{name}.py"));
    let code = fs::read_to_string(&path).ok()?;
    Some(ArsenalEntry {
        name: name.to_string(),
        code,
        created_by_model: String::new(),
        uses: 0,
    })
}

/// Invoke a stored arsenal entry via a restricted Python subprocess.
/// The function file is loaded with `python -c`, the entry's code
/// inlined. Output is capped at [`OUTPUT_CAP`] bytes (10KB). No
/// imports beyond the safe set (json, csv, re, math, pathlib, os.path
/// read-only).
///
/// On error (missing file, python failure, non-utf8 output) returns
/// an error string — never panics.
pub fn invoke(arsenal_dir: &Path, name: &str, args: &[String]) -> String {
    let path = arsenal_dir.join(format!("{name}.py"));
    let code = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) => return format!("error: entry '{name}' not found: {e}"),
    };

    // Inline the entry's code; argv passed as a JSON array the
    // function can read from `_argv`. No imports beyond the safe set.
    let runner = format!(
        "import json as _json\n_argv = {argv}\n{code}\n",
        argv = serde_json_array(args),
    );

    let out = match Command::new("python").arg("-c").arg(&runner).output() {
        Ok(o) => o,
        Err(e) => return format!("error: python failed to start: {e}"),
    };

    let mut text = String::from_utf8_lossy(&out.stdout).into_owned();
    if out.status.code() != Some(0) {
        let err = String::from_utf8_lossy(&out.stderr);
        return format!(
            "error: python exited {:?}: {}",
            out.status.code(),
            err.trim()
        );
    }
    if text.len() > OUTPUT_CAP {
        text.truncate(OUTPUT_CAP);
    }
    text
}

/// Build a JSON array literal string from the args (minimal JSON
/// stringifier — escapes double-quotes and backslashes). Zero-dep,
/// same house style as [`crate::deja_vu`] substring readers.
fn serde_json_array(args: &[String]) -> String {
    let parts: Vec<String> = args
        .iter()
        .map(|a| {
            let esc = a.replace('\\', "\\\\").replace('"', "\\\"");
            format!("\"{esc}\"")
        })
        .collect();
    format!("[{}]", parts.join(", "))
}

/// Entries with `uses == 0` are composting candidates — the déjà-vu
/// organ measures stagnation across [`STAGNANT_WINDOW`] epochs; the
/// host disposes. Pure: no I/O, reads only the slice it is given.
pub fn compost_candidates(entries: &[ArsenalEntry]) -> Vec<&ArsenalEntry> {
    entries.iter().filter(|e| e.uses == 0).collect()
}
