//! page mark report tests — CARD-0195. Hermetic HOME.
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-pmark-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    home: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let home = tmp(tag).join("home");
        fs::create_dir_all(&home).unwrap();
        Self { home }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        self.run_env(args, None)
    }

    fn run_env(&self, args: &[&str], mark_env: Option<&str>) -> (String, String, i32) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis"));
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env_remove("CADDIS_PAGE_MARK_TOKENS");
        if let Some(v) = mark_env {
            cmd.env("CADDIS_PAGE_MARK_TOKENS", v);
        }
        let mut child = cmd.spawn().expect("spawn caddis");
        child.stdin.as_mut().unwrap().write_all(b"").unwrap();
        let out = child.wait_with_output().expect("caddis must finish");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }
}

fn seed_log(home: &std::path::Path) -> PathBuf {
    let pager = home.join(".caddis").join("pager");
    fs::create_dir_all(&pager).unwrap();
    fs::write(
        pager.join("s1.observe.jsonl"),
        "{\"kind\":\"context\",\"parse_ok\":true,\"stored_tokens\":1,\"sent_est_tokens\":1}\n",
    )
    .unwrap();
    pager
}

#[test]
fn report_prints_mark_from_file() {
    let w = World::new("mark");
    let pager = seed_log(&w.home);
    let (o, e, c) = w.run(&["page", "report", "--session", "s1"]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(!o.lines().any(|l| l.starts_with("mark=")), "{o}");
    assert!(!o.lines().any(|l| l.starts_with("sent_mark_milli=")), "{o}");
    assert!(
        !o.lines().any(|l| l.starts_with("stored_mark_milli=")),
        "{o}"
    );
    fs::write(pager.join("mark"), "20000\n").unwrap();
    let (o, e, c) = w.run(&["page", "report", "--session", "s1"]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("mark=20000"), "{o}");
}

#[test]
fn report_mark_env_beats_file() {
    let w = World::new("markenv");
    let pager = seed_log(&w.home);
    fs::write(pager.join("mark"), "20000\n").unwrap();
    let (o, e, c) = w.run_env(&["page", "report", "--session", "s1"], Some("15000"));
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("mark=15000"), "{o}");
    assert!(!o.contains("mark=20000"), "{o}");
}

#[test]
fn report_prints_sent_mark_milli() {
    let w = World::new("smm");
    let pager = w.home.join(".caddis").join("pager");
    fs::create_dir_all(&pager).unwrap();
    fs::write(
        pager.join("s1.observe.jsonl"),
        "{\"kind\":\"context\",\"parse_ok\":true,\"stored_tokens\":80000,\"sent_est_tokens\":90000}\n",
    )
    .unwrap();
    fs::write(pager.join("mark"), "20000\n").unwrap();
    let (o, e, c) = w.run(&["page", "report", "--session", "s1"]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("mark=20000"), "{o}");
    assert!(o.contains("sent_mark_milli=4500"), "{o}");
    assert!(o.contains("stored_mark_milli=4000"), "{o}");
}

#[test]
fn report_prints_last_n_evicted() {
    let w = World::new("nev");
    let pager = seed_log(&w.home);
    let (o, e, c) = w.run(&["page", "report", "--session", "s1"]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("last_n_evicted=-1"), "{o}");
    fs::write(
        pager.join("s1.observe.jsonl"),
        "{\"kind\":\"context\",\"parse_ok\":true,\"stored_tokens\":1,\"sent_est_tokens\":1}\n{\"kind\":\"project\",\"n_evicted\":12}\n",
    )
    .unwrap();
    let (o, e, c) = w.run(&["page", "report", "--session", "s1"]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("last_n_evicted=12"), "{o}");
}

#[test]
fn page_mark_set_writes_session_file() {
    let w = World::new("set");
    let (o, e, c) = w.run(&["page", "mark", "--session", "s1", "--set", "25000"]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("mark=25000"), "{o}");
    let p = w.home.join(".caddis").join("pager").join("s1").join("mark");
    assert_eq!(fs::read_to_string(p).unwrap().trim(), "25000");
}

#[test]
fn page_mark_no_set_prints_session_or_zero() {
    let w = World::new("noset");
    let (o, e, c) = w.run(&["page", "mark", "--session", "s1"]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("mark=0"), "{o}");
    let (o, e, c) = w.run(&["page", "mark", "--session", "s1", "--set", "9000"]);
    assert_eq!(c, 0, "{o}{e}");
    let (o, e, c) = w.run(&["page", "mark", "--session", "s1"]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("mark=9000"), "{o}");
}

#[test]
fn page_mark_set_rejects_zero() {
    let w = World::new("zero");
    let (o, e, c) = w.run(&["page", "mark", "--session", "s1", "--set", "0"]);
    assert_eq!(c, 2, "{o}{e}");
}

#[test]
fn page_mark_set_rejects_non_int() {
    let w = World::new("noint");
    let (o, e, c) = w.run(&["page", "mark", "--session", "s1", "--set", "nope"]);
    assert_eq!(c, 2, "{o}{e}");
}

#[test]
fn session_mark_beats_global() {
    let w = World::new("sess");
    let pager = seed_log(&w.home);
    fs::write(pager.join("mark"), "20000\n").unwrap();
    let (o, e, c) = w.run(&["page", "mark", "--session", "s1", "--set", "15000"]);
    assert_eq!(c, 0, "{o}{e}");
    let (o, e, c) = w.run(&["page", "report", "--session", "s1"]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("mark=15000"), "{o}");
    assert!(!o.contains("mark=20000"), "{o}");
}

#[test]
fn report_prints_last_role_chars() {
    let w = World::new("chars");
    let pager = seed_log(&w.home);
    let path = pager.join("s1.observe.jsonl");
    let mut body = fs::read_to_string(&path).unwrap();
    body.push_str(
        "{\"kind\":\"context\",\"parse_ok\":true,\"user_chars\":27,\"assistant_chars\":17,\"toolResult_chars\":4000}\n",
    );
    fs::write(&path, body).unwrap();
    let (o, e, c) = w.run(&["page", "report", "--session", "s1"]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("last_user_chars=27"), "{o}");
    assert!(o.contains("last_assistant_chars=17"), "{o}");
    assert!(o.contains("last_tool_result_chars=4000"), "{o}");
}
