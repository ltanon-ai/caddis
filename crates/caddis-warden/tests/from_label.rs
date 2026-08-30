//! from_label.rs — CARD-FROM-1: the ledger's `from` field names the CALLING
//! harness, and one conscience now serves four of them (omp, little-coder,
//! prime-agent, Claude Code). It was hardcoded "omp", so every harness's
//! verdicts were attributed to omp — the shared ledger could not answer
//! "which of my agents did this", which is the only question a shared ledger
//! exists to answer. The caller id moves to CADDIS_WARDEN_FROM, sanitized to
//! the envelope-legal charset so a hostile value cannot corrupt the JSONL row.

use std::io::Write;
use std::process::{Command, Stdio};

/// The adapter's frame: `<name> <bytelen>\n<bytes>\n` per field, in the fixed
/// order the wire parser reads. Byte lengths, not char counts — the wire law.
fn frame(tool: &str, command: &str) -> Vec<u8> {
    let mut v = Vec::new();
    for (name, body) in [
        ("tool", tool),
        ("command", command),
        ("path", ""),
        ("content", ""),
    ] {
        v.extend_from_slice(format!("{name} {}\n", body.len()).as_bytes());
        v.extend_from_slice(body.as_bytes());
        v.extend_from_slice(b"\n");
    }
    v
}

/// Run the binary once against a throwaway ledger; return its stdout reply and
/// the ledger text it left behind. Each call gets its own ledger file so two
/// tests in one process cannot read each other's rows.
fn judge(from: Option<&str>, tag: &str) -> (String, String) {
    let ledger =
        std::env::temp_dir().join(format!("caddis-from-{}-{tag}.jsonl", std::process::id()));
    let _ = std::fs::remove_file(&ledger);
    let mut builder = Command::new(env!("CARGO_BIN_EXE_caddis-warden"));
    builder.env("CADDIS_WARDEN_LEDGER", &ledger);
    if let Some(f) = from {
        builder.env("CADDIS_WARDEN_FROM", f);
    }
    let mut cmd = builder
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("spawn warden binary");
    cmd.stdin
        .take()
        .expect("stdin")
        .write_all(&frame("bash", "echo hi"))
        .expect("write frame");
    let out = cmd.wait_with_output().expect("warden exits");
    let reply = String::from_utf8_lossy(&out.stdout).into_owned();
    let rows = std::fs::read_to_string(&ledger).unwrap_or_default();
    let _ = std::fs::remove_file(&ledger);
    (reply, rows)
}

#[test]
fn from_env_names_the_calling_harness() {
    let (reply, rows) = judge(Some("little-coder"), "env");
    assert!(
        reply.contains("\"verdict\""),
        "a readable reply came back: {reply}"
    );
    assert!(
        rows.contains("\"from\":\"little-coder\""),
        "the row must attribute the call to the env-named caller, got: {rows}"
    );
}

#[test]
fn from_default_stays_omp_for_unset_env() {
    let (_reply, rows) = judge(None, "default");
    assert!(
        rows.contains("\"from\":\"omp\""),
        "no env, no surprise: the caller stays omp, got: {rows}"
    );
}

#[test]
fn from_env_is_sanitized_to_the_envelope_charset() {
    // A newline or quote in the caller id would corrupt the JSONL row — the
    // `evil\nid"x` keeps exactly [evil]+[id]+[x] after the charset filter.
    let (_reply, rows) = judge(Some("evil\nid\"x"), "dirty");
    assert!(
        rows.contains("\"from\":\"evilidx\""),
        "illegal bytes are dropped, the row stays one line, got: {rows}"
    );
    assert_eq!(
        rows.lines().count(),
        1,
        "exactly one ledger row, no injection"
    );
}
