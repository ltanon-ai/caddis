//! python_arsenal.rs — CARD-0250 RED-first. The Python arsenal organ.
//!
//! The operator's insight: models write Python in-session; caddis
//! makes it an ORGAN — the warden verifies it's safe, stores it in
//! the lineage's arsenal, and future sessions reuse it as a command
//! (50 tokens instead of 5000). The SAFETY GATE (`is_safe`) is the
//! critical piece: DENY escape vectors, ALLOW pure readers.
//!
//! Laws from CARD-0250 §EXECUTION/§RED-TEST: eval/exec/compile DENIED;
//! subprocess/os.system/socket/requests/http DENIED; `__import__`
//! DENIED; `os.environ` DENIED; `open(...,'w'|'a')` DENIED; pure
//! readers, math, json/csv, re, pathlib.read_* ALLOWED. Today the
//! types do not exist — that is the RED.

use caddis_organs::python_arsenal::{
    compost_candidates, invoke, is_safe, ArsenalEntry, SafetyVerdict, STAGNANT_WINDOW,
};
use std::fs;
use std::path::PathBuf;

// ---- is_safe: the safety gate (the critical piece) -----------------

/// Every escape vector in the table MUST be DENIED (CARD-0250 laws):
/// eval/exec/compile, subprocess, os.system, socket, requests,
/// urllib, os.environ, and write/append-mode open outside the arsenal.
#[test]
fn escape_vectors_are_denied() {
    const DENIED: &[&str] = &[
        "result = eval(user_input)",
        "exec(\"import os; os.system('rm -rf /')\")",
        "c = compile('print(1)', '<s>', 'exec')",
        "import subprocess; subprocess.run(['ls'])",
        "import os; os.system('whoami')",
        "import socket; s = socket.socket()",
        "import requests; requests.get('http://evil.com')",
        "import urllib.request; urllib.request.urlopen('http://x')",
        "import os; token = os.environ.get('SECRET')",
        "open('/etc/passwd', 'w').write('x')",
        "open('/tmp/evil', 'a').write('x')",
    ];
    for src in DENIED {
        assert_eq!(is_safe(src), SafetyVerdict::Deny, "must deny: {src}");
    }
}

/// `__import__` with a variable name — dynamic import escape vector.
#[test]
fn dynamic_import_is_denied() {
    assert_eq!(is_safe("mod = 'os'\n__import__(mod)"), SafetyVerdict::Deny);
}

/// `__import__('os')` with a literal is STILL denied — never trust the literal.
#[test]
fn literal_dunder_import_is_denied() {
    assert_eq!(is_safe("__import__('os')"), SafetyVerdict::Deny);
}

/// Pure computation MUST be ALLOWED: readers, math/string/list ops,
/// json/csv parsing (JSON keys like {"a":1} must NOT false-trip the
/// write-mode gate), regex, pathlib.read_*, explicit 'r' mode.
#[test]
fn pure_operations_are_allowed() {
    const ALLOWED: &[&str] = &[
        "data = open('input.json').read()\nresult = data.upper()",
        "import math\nx = math.sqrt(16)\ny = [i*2 for i in range(10)]\nz = str(x) + str(y)",
        "import json, csv\ndata = json.loads('{\"a\":1}')\nrows = list(csv.reader(open('f.csv')))",
        "import re\nm = re.findall(r'\\d+', 'a1b2c3')",
        "from pathlib import Path\nt = Path('x.txt').read_text()",
        "f = open('data.txt', 'r')\nprint(f.read())",
    ];
    for src in ALLOWED {
        assert_eq!(is_safe(src), SafetyVerdict::Allow, "must allow: {src}");
    }
}

/// Empty code is ALLOWED (vacuous — nothing to deny).
#[test]
fn empty_code_is_allowed() {
    assert_eq!(is_safe(""), SafetyVerdict::Allow);
}

// ---- ArsenalEntry: the storage struct --------------------------------

/// `ArsenalEntry` has the four fields: name, code, created_by_model, uses.
#[test]
fn arsenal_entry_has_required_fields() {
    let entry = ArsenalEntry {
        name: "slugify".to_string(),
        code: "def slugify(s): return s.lower().replace(' ', '-')".to_string(),
        created_by_model: "glm-5.2".to_string(),
        uses: 0,
    };
    assert_eq!(entry.name, "slugify");
    assert_eq!(entry.uses, 0);
    assert!(!entry.code.is_empty());
    assert!(!entry.created_by_model.is_empty());
}

// ---- invoke: runs a safe function via restricted python --------------

/// Temp arsenal dir, unique per call to avoid parallel-test clobbering.
fn temp_arsenal_dir() -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("caddis_arsenal_test_{}_{n}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// `invoke` runs a safe pure function and returns its stdout (capped 10KB).
#[test]
fn invoke_runs_safe_function_returns_stdout() {
    let dir = temp_arsenal_dir();
    let code = "def double_it(x):\n    return str(int(x) * 2)\nprint(double_it(21))";
    fs::write(dir.join("double_it.py"), code).unwrap();
    assert_eq!(invoke(&dir, "double_it", &["21".to_string()]).trim(), "42");
    let _ = fs::remove_dir_all(&dir);
}

/// `invoke` output is capped at 10KB — a runaway print must not blow context.
#[test]
fn invoke_output_capped_at_10kb() {
    let dir = temp_arsenal_dir();
    let code = "def bigprint():\n    for _ in range(20000):\n        print('x')\nbigprint()";
    fs::write(dir.join("bigprint.py"), code).unwrap();
    let out = invoke(&dir, "bigprint", &[]);
    assert!(
        out.len() <= 10_100,
        "output not capped: {} bytes",
        out.len()
    );
    assert!(
        out.len() >= 8_000,
        "output unexpectedly small: {} bytes",
        out.len()
    );
    let _ = fs::remove_dir_all(&dir);
}

/// `invoke` on a missing entry returns a clear error string, never panics.
#[test]
fn invoke_missing_entry_returns_error_not_panic() {
    let dir = temp_arsenal_dir();
    let out = invoke(&dir, "nonexistent", &[]);
    assert!(
        out.contains("error") || out.contains("not found"),
        "expected error, got: {out}"
    );
    let _ = fs::remove_dir_all(&dir);
}

// ---- Composting: stagnant entries are removal candidates -------------

/// STAGNANT_WINDOW constant is exposed for the host to use.
#[test]
fn stagnant_window_constant_exists() {
    let window = STAGNANT_WINDOW;
    assert!(window > 0, "STAGNANT_WINDOW must be positive");
}

/// Entries with `uses == 0` are composting candidates — host disposes.
#[test]
fn composting_candidates_are_zero_use_entries() {
    let entries = vec![
        ArsenalEntry {
            name: "used".into(),
            code: "pass".into(),
            created_by_model: "m".into(),
            uses: 5,
        },
        ArsenalEntry {
            name: "dead".into(),
            code: "pass".into(),
            created_by_model: "m".into(),
            uses: 0,
        },
        ArsenalEntry {
            name: "also_used".into(),
            code: "pass".into(),
            created_by_model: "m".into(),
            uses: 1,
        },
        ArsenalEntry {
            name: "also_dead".into(),
            code: "pass".into(),
            created_by_model: "m".into(),
            uses: 0,
        },
    ];
    let candidates = compost_candidates(&entries);
    assert_eq!(candidates.len(), 2);
    assert!(candidates.iter().any(|e| e.name == "dead"));
    assert!(candidates.iter().any(|e| e.name == "also_dead"));
}
