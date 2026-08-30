//! page.rs tests — CARD-0155. Hermetic HOME. Line protocol, zero-dep.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-page-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn hex(s: &str) -> String {
    s.bytes().map(|b| format!("{b:02x}")).collect()
}

struct World {
    home: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        Self { home }
    }

    fn run_stdin(&self, args: &[&str], stdin: &str) -> (String, String, i32) {
        let mut child = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .spawn()
            .expect("spawn caddis");
        child
            .stdin
            .as_mut()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        let out = child.wait_with_output().expect("caddis must finish");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn capture(&self, seq: u32, turn: u32, body: &str) {
        let stdin = format!(
            "seq={}\nrole=toolResult\nchars={}\nturn={}\ntext={}\n",
            seq,
            body.len(),
            turn,
            hex(body)
        );
        let (o, e, c) = self.run_stdin(&["page", "capture", "--session", "s1"], &stdin);
        assert_eq!(c, 0, "capture {seq}: {o}{e}");
    }
}

#[test]
fn capture_ref_roundtrip_preserves_multiline_verbatim() {
    let w = World::new("round");
    let body = "line one\n{\n  \"deep\": [1, 2, 3]\n}\nline four — em dash ✓";
    w.capture(1, 2, body);
    w.capture(2, 5, "second body");
    // idempotent re-capture
    w.capture(1, 2, body);
    let (o, e, c) = w.run_stdin(&["page", "ref", "--session", "s1", "--seq", "1"], "");
    assert_eq!(c, 0, "ref: {o}{e}");
    assert_eq!(o, body, "verbatim roundtrip");
    let store = fs::read_to_string(
        w.home
            .join(".caddis")
            .join("pager")
            .join("s1")
            .join("cold.store"),
    )
    .unwrap();
    assert_eq!(store.matches("seq=1\n").count(), 1, "no duplicate seq");
    assert!(store.contains("sha="), "checksum recorded");
    let (_o, _e, c) = w.run_stdin(&["page", "ref", "--session", "s1", "--seq", "9"], "");
    assert_ne!(c, 0, "absent seq must fail");
}

#[test]
fn tick_stages_protect_and_rank() {
    let w = World::new("tick");
    // turn 100 is "now": working set = turns 95..100
    let spans = "1,10,4000,0\n2,10,900,0\n3,40,5000,0\n4,50,8000,1\n5,97,9000,0\n6,20,100,0\n";
    // expect: seq6 (below evict_min) never; seq5 working set never; seq4 pinned never;
    // oldest band = turn 10: seq1 (4000) before seq2 (900); then seq3 (turn 40).
    let (o, e, c) = w.run_stdin(
        &["page", "tick", "--session", "s1", "--mark-tokens", "900"],
        spans,
    );
    assert_eq!(c, 0, "tick: {o}{e}");
    let lines: Vec<&str> = o.lines().collect();
    assert!(
        lines.contains(&"evict=1"),
        "oldest band first, larger first: {o}"
    );
    assert!(lines.contains(&"evict=2"), "same turn, size tiebreak: {o}");
    assert!(
        lines.contains(&"evict=3"),
        "next band once the first is spent: {o}"
    );
    assert!(!o.contains("evict=4"), "pinned never: {o}");
    assert!(!o.contains("evict=5"), "working set never: {o}");
    assert!(!o.contains("evict=6"), "below evict_min never: {o}");
    assert!(o.contains("starved="), "starred line present: {o}");
}

#[test]
fn tick_below_mark_evicts_nothing_and_cap_starves() {
    let w = World::new("quiet");
    let (o, _e, c) = w.run_stdin(
        &["page", "tick", "--session", "s1", "--mark-tokens", "9000"],
        "1,10,4000,0\n",
    );
    assert_eq!(c, 0);
    assert!(!o.contains("evict="), "below mark evicts nothing: {o}");
    assert!(o.contains("starved=false"), "{o}");

    let (o, _e, c) = w.run_stdin(
        &[
            "page",
            "tick",
            "--session",
            "s1",
            "--mark-tokens",
            "1",
            "--cap",
            "1",
        ],
        "1,1,4000,0\n2,2,4000,0\n9,50,10,0\n",
    );
    assert_eq!(c, 0);
    assert_eq!(o.matches("evict=").count(), 1, "cap bounds the cycle: {o}");
    assert!(o.contains("starved=true"), "cap blocks the mark: {o}");
}

#[test]
fn help_names_page_report() {
    let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
        .arg("--help")
        .output()
        .unwrap();
    let s = String::from_utf8_lossy(&out.stdout);
    assert!(
        s.contains("page tick [--session")
            && s.contains("page mode --session")
            && s.contains("page mark --session"),
        "{s}"
    );
}

#[test]
fn tick_ceil_chars_matches_nerve() {
    let w = World::new("ceil");
    let (o, e, c) = w.run_stdin(
        &[
            "page",
            "tick",
            "--session",
            "s1",
            "--mark-tokens",
            "1",
            "--keep-recent",
            "0",
            "--evict-min",
            "1",
        ],
        "1,1,5,0\n",
    );
    assert_eq!(c, 0, "tick: {o}{e}");
    assert!(
        o.contains("evict=1"),
        "5 chars is 2 tokens ceil, mark 1: {o}"
    );
}

#[test]
fn tick_without_session_still_ranks() {
    let w = World::new("nosess");
    let (o, e, c) = w.run_stdin(
        &[
            "page",
            "tick",
            "--mark-tokens",
            "1",
            "--keep-recent",
            "0",
            "--evict-min",
            "1",
        ],
        "1,1,5,0\n",
    );
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("evict=1"), "{o}");
}
