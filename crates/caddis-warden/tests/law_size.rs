//! CARD-ALL-TOOLCALL-1 — 280-line files and CCN-10 functions are DENY
//! at the tool call. A bee's `from` is the harness, not the parent.

use std::io::Write;
use std::process::{Command, Stdio};

use caddis_warden::{decide, ToolCall, Verdict};

fn lines(n: usize) -> String {
    (0..n)
        .map(|i| format!("const C{i}: u8 = 0;"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn ifs(n: usize) -> String {
    let mut s = String::from("fn f(a: bool) {\n");
    for _ in 0..n {
        s.push_str("    if a { let _x = 1; }\n");
    }
    s.push_str("}\n");
    s
}

fn is_deny_for(reason_needle: &str, call: ToolCall) -> String {
    match decide(&call) {
        Verdict::Deny { reason } => {
            assert!(
                reason.contains(reason_needle),
                "deny reason {reason} should name {reason_needle}"
            );
            reason
        }
        other => panic!("expected Deny, got {other:?}"),
    }
}

#[test]
fn write_of_281_lines_is_deny() {
    is_deny_for(
        "280",
        ToolCall::new("write").path("src/n.rs").content(&lines(281)),
    );
}

#[test]
fn write_of_280_lines_is_not_size_deny() {
    let v = decide(&ToolCall::new("write").path("src/n.rs").content(&lines(280)));
    if let Verdict::Deny { reason } = v {
        assert!(
            !reason.contains("280") && !reason.contains("CCN"),
            "280 lines is the cap, not over it: {reason}"
        );
    }
}

#[test]
fn edit_of_existing_281_line_file_is_deny() {
    let dir = std::env::temp_dir().join(format!("caddis-size-edit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("big.rs");
    std::fs::write(&path, lines(281)).unwrap();
    is_deny_for(
        "280",
        ToolCall::new("edit")
            .path(path.to_str().unwrap())
            .content("const C: u8 = 1;"),
    );
}

#[test]
fn function_ccn_11_is_deny() {
    is_deny_for(
        "CCN",
        ToolCall::new("write").path("src/f.rs").content(&ifs(10)),
    );
}

#[test]
fn function_ccn_10_is_not_ccn_deny() {
    let v = decide(&ToolCall::new("write").path("src/f.rs").content(&ifs(9)));
    if let Verdict::Deny { reason } = v {
        assert!(!reason.contains("CCN"), "CCN 10 is the cap: {reason}");
    }
}

fn write_frame(path: &str, content: &str) -> Vec<u8> {
    let mut v = Vec::new();
    for (name, body) in [
        ("tool", "write"),
        ("command", ""),
        ("path", path),
        ("content", content),
    ] {
        v.extend_from_slice(format!("{name} {}\n", body.len()).as_bytes());
        v.extend_from_slice(body.as_bytes());
        v.extend_from_slice(b"\n");
    }
    v
}

#[test]
fn bee_from_281_write_is_ledgered_deny() {
    let ledger = std::env::temp_dir().join(format!(
        "caddis-size-from-{}-{}.jsonl",
        std::process::id(),
        "bee"
    ));
    let _ = std::fs::remove_file(&ledger);
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis-warden"));
    cmd.env("CADDIS_WARDEN_LEDGER", &ledger)
        .env("CADDIS_WARDEN_FROM", "omp")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn().expect("spawn warden");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(&write_frame("src/n.rs", &lines(281)))
        .unwrap();
    let out = child.wait_with_output().expect("warden exits");
    let reply = String::from_utf8_lossy(&out.stdout);
    let rows = std::fs::read_to_string(&ledger).unwrap_or_default();
    let _ = std::fs::remove_file(&ledger);
    assert!(
        reply.to_ascii_lowercase().contains("deny"),
        "warden must deny the 281-line write: {reply}"
    );
    assert!(
        rows.contains("\"from\":\"omp\""),
        "ledger from is the bee harness, got: {rows}"
    );
}
