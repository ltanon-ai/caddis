//! replay.rs — CARD-REPLAY-1: the ledger is not just memory, it is a
//! simulator. `caddis-warden --replay <ledger>` re-judges every recorded
//! command against the CURRENT law and reports the diff — a law change you
//! can preview against your own history before it ever guards a live agent.
//! These tests pin the contract at the binary level.

use std::process::{Command, Stdio};

fn row(seq: u64, body: &str) -> String {
    format!(
        "{{\"seq\":{seq},\"v\":1,\"id\":\"x{seq:016}\",\"idem_key\":\"k{seq}\",\
         \"type\":\"tool.bash\",\"from\":\"t\",\"to\":\"warden\",\"body\":{body:?},\"ts\":1}}\n",
        seq = seq,
        body = body
    )
}

fn write_ledger(tag: &str) -> std::path::PathBuf {
    let path =
        std::env::temp_dir().join(format!("caddis-replay-{}-{tag}.jsonl", std::process::id()));
    let rows = format!(
        "{}{}{}{}",
        row(1, "allow|echo ok||"),
        row(2, "deny|git push --force origin main||why"),
        row(3, "allow|git push --force origin main||"), // old law missed it
        row(4, "deny|echo harmless||why"),              // old law overfired
    );
    std::fs::write(&path, rows).unwrap();
    path
}

fn replay(path: &std::path::Path) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_caddis-warden"))
        .arg("--replay")
        .arg(path)
        .stdin(Stdio::null())
        .output()
        .expect("spawn warden");
    String::from_utf8_lossy(&out.stdout).into_owned()
}

#[test]
fn replay_reports_the_divergence_between_old_and_new_law() {
    let path = write_ledger("diff");
    let report = replay(&path);
    let _ = std::fs::remove_file(&path);
    assert!(
        report.contains("new-denies: 1"),
        "one recorded allow that today's law denies, got: {report}"
    );
    assert!(
        report.contains("freed: 1"),
        "one recorded deny that today's law allows, got: {report}"
    );
    assert!(
        report.contains("NEW-DENY seq=3"),
        "the divergent row is named with its seq, got: {report}"
    );
    assert!(
        report.contains("FREED   seq=4"),
        "the freed row is named with its seq, got: {report}"
    );
}

#[test]
fn replay_of_a_current_law_ledger_reports_no_divergence() {
    // rows exactly as the current engine would write them -> identity
    let path =
        std::env::temp_dir().join(format!("caddis-replay-{}-ident.jsonl", std::process::id()));
    std::fs::write(
        &path,
        format!(
            "{}{}",
            row(1, "allow|echo ok||"),
            row(2, "deny|git push --force origin main||why")
        ),
    )
    .unwrap();
    let report = replay(&path);
    let _ = std::fs::remove_file(&path);
    assert!(
        report.contains("new-denies: 0") && report.contains("freed: 0"),
        "identity: replaying what the current law wrote changes nothing, got: {report}"
    );
}

#[test]
fn replay_skips_what_the_ledger_deliberately_never_kept() {
    let path =
        std::env::temp_dir().join(format!("caddis-replay-{}-skip.jsonl", std::process::id()));
    std::fs::write(
        &path,
        format!(
            "{}{}",
            row(1, "allow|echo ***redacted(len=35)||"),
            "{\"seq\":2,\"v\":1,\"id\":\"y\",\"idem_key\":\"k\",\"type\":\"tool.write\",\"from\":\"t\",\"to\":\"warden\",\"body\":\"allow||/tmp/f.py|\",\"ts\":1}\n"
        ),
    )
    .unwrap();
    let report = replay(&path);
    let _ = std::fs::remove_file(&path);
    assert!(
        report.contains("skipped: 2"),
        "masked commands and non-command tools are skipped, not guessed, got: {report}"
    );
}
