//! verify_cli.rs — dispatch regression, through the real binary.
//!
//! da0f3c8 (2026-08-27, "house the spawn organ in the OS workspace")
//! silently deleted the verify wiring — the `pub mod verify;`, the import,
//! and the dispatch arm — in an unrelated alphabetical hunk. The findings
//! engine and its 8 tests stayed green in memory while the SHIPPED surface
//! answered "unknown argument". A surface nobody re-tests through argv is a
//! surface that quietly stops existing; this file is the pin.

use std::process::{Command, Stdio};

fn tmp(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "caddis-verify-cli-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ))
}

fn verify(ledger: &std::path::Path, extra: &[&str]) -> (String, String, i32) {
    let out = Command::new(env!("CARGO_BIN_EXE_caddis-warden"))
        .arg("verify")
        .arg(ledger)
        .args(extra)
        .env("CADDIS_WARDEN_LEDGER", ledger)
        .stdin(Stdio::null())
        .output()
        .expect("the binary must spawn");
    (
        String::from_utf8_lossy(&out.stdout).into_owned(),
        String::from_utf8_lossy(&out.stderr).into_owned(),
        out.status.code().unwrap_or(-1),
    )
}

#[test]
fn verify_answers_as_a_subcommand_and_codes_findings() {
    let ledger = tmp("led").with_extension("jsonl");
    // one healthy row, one row whose from-label swallowed another writer's
    // row head (the weak-L1 fusion shape) — both parse, one is a finding.
    let clean = "{\"seq\":1,\"v\":11,\"type\":\"tool.bash\",\
                 \"body\":\"allow|echo a||\",\"from\":\"omp\",\"ts\":10,\"id\":\"i1\"}";
    let junk = "{\"seq\":2,\"v\":11,\"type\":\"tool.bash\",\
                \"body\":\"allow|echo b||\",\"from\":\"omp{\",\"ts\":20,\"id\":\"i2\"}";
    std::fs::write(&ledger, format!("{clean}\n{junk}\n")).expect("fixture written");

    let (out, err, code) = verify(&ledger, &[]);
    let _ = std::fs::remove_file(&ledger);
    assert_eq!(code, 3, "findings exit 3 through argv: {out}{err}");
    assert!(
        !out.contains("unknown argument"),
        "the dispatch arm must exist: {out}{err}"
    );
    assert!(out.contains("junk from: 1 values"), "{out}");
    assert!(out.contains("status: FINDINGS"), "{out}");

    // --json carries the machine-readable finding counts.
    std::fs::write(&ledger, format!("{clean}\n{junk}\n")).expect("fixture rewritten");
    let (jout, _, jcode) = verify(&ledger, &["--json"]);
    let _ = std::fs::remove_file(&ledger);
    assert_eq!(jcode, 3, "{jout}");
    assert!(jout.contains("\"junk_from_values\":1"), "{jout}");
    assert!(jout.contains("\"status\":\"findings\""), "{jout}");
}

#[test]
fn a_clean_ledger_exits_zero_through_argv() {
    let ledger = tmp("ok").with_extension("jsonl");
    std::fs::write(
        &ledger,
        "{\"seq\":1,\"v\":11,\"type\":\"tool.bash\",\
          \"body\":\"allow|echo a||\",\"from\":\"omp\",\"ts\":10,\"id\":\"i1\"}\n",
    )
    .expect("fixture written");
    let (out, err, code) = verify(&ledger, &[]);
    let _ = std::fs::remove_file(&ledger);
    assert_eq!(code, 0, "{out}{err}");
    assert!(out.contains("status: CLEAN"), "{out}");
}
