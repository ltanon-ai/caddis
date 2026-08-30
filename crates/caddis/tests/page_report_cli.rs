//! page_report_cli.rs — report tests split from page.rs (280 cap).

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
}

#[test]
fn report_digests_canary_lines_from_observe_log() {
    let w = World::new("report");
    let pager = w.home.join(".caddis").join("pager");
    fs::create_dir_all(&pager).unwrap();
    fs::write(
        pager.join("s1.observe.jsonl"),
        "{\"kind\":\"context\",\"parse_ok\":true,\"n_messages\":10,\"chars\":3000,\"largest_tool_result_chars\":2000,\"stored_tokens\":50000,\"sent_est_tokens\":48000,\"stored_pct\":2,\"stored_window\":500000}\n\
         {\"kind\":\"context\",\"parse_ok\":true,\"n_messages\":11,\"chars\":4000,\"largest_tool_result_chars\":3500,\"roles\":{\"user\":2,\"assistant\":3,\"toolResult\":5,\"custom\":4},\"stored_tokens\":52000,\"sent_est_tokens\":49000,\"stored_pct\":3,\"stored_window\":500000,\"page_mode\":true,\"n_stubbed\":7}\n\
         {\"kind\":\"context\",\"parse_ok\":false}\n\
         {\"kind\":\"compact_before\"}\n\
         {\"kind\":\"compact_auto_start\"}\n\
         {\"kind\":\"compact_auto_end\"}\n\
         {\"kind\":\"message_end\",\"usage\":{\"cost\":{\"input\":1},\"input\":9000,\"cacheRead\":7000,\"output\":50,\"reasoningTokens\":12,\"cacheWrite\":0}}\n",
    )
    .unwrap();
    let (o, e, c) = w.run_stdin(&["page", "report", "--session", "s1"], "");
    assert_eq!(c, 0, "report: {o}{e}");
    assert!(o.contains("context_events=3"), "{o}");
    assert!(o.contains("parse_fail=1"), "{o}");
    assert!(o.contains("parse_fail_milli=333"), "{o}");
    assert!(o.contains("compact_before=1"), "{o}");
    assert!(o.contains("compact_auto=1"), "{o}");
    assert!(o.contains("input=9000"), "{o}");
    assert!(o.contains("cacheRead=7000"), "{o}");
    assert!(o.contains("output=50"), "{o}");
    assert!(o.contains("reasoningTokens=12"), "{o}");
    assert!(o.contains("cacheWrite=0"), "{o}");
    assert!(o.contains("last_sent_est=49000"), "{o}");
    assert!(o.contains("last_stored=52000"), "{o}");
    assert!(o.contains("stored_sent_milli=1061"), "{o}");
    assert!(o.contains("last_pct=3"), "{o}");
    assert!(o.contains("last_window=500000"), "{o}");
    assert!(
        o.contains("stored_window_milli=104") && o.contains("fold_headroom_milli=796"),
        "{o}"
    );
    assert!(o.contains("last_n_messages=11"), "{o}");
    assert!(o.contains("last_chars=4000"), "{o}");
    assert!(o.contains("last_largest_tool=3500"), "{o}");
    assert!(o.contains("last_custom=4"), "{o}");
    assert!(o.contains("last_user=2"), "{o}");
    assert!(o.contains("last_assistant=3"), "{o}");
    assert!(o.contains("last_tool_result=5"), "{o}");
    assert!(o.contains("last_page_mode=true"), "{o}");
    assert!(
        o.contains("last_n_stubbed=7") && o.contains("stub_tool_milli=1400"),
        "{o}"
    );
    let (o, e, c) = w.run_stdin(&["page", "report", "--session", "nope"], "");
    assert_ne!(c, 0, "missing log must fail: {o}{e}");
}

#[test]
fn report_without_session_uses_newest_observe_log() {
    let w = World::new("latest");
    let pager = w.home.join(".caddis").join("pager");
    fs::create_dir_all(&pager).unwrap();
    fs::write(
        pager.join("aaa.observe.jsonl"),
        "{\"kind\":\"context\",\"parse_ok\":true,\"stored_tokens\":111,\"sent_est_tokens\":100}\n",
    )
    .unwrap();
    fs::write(
        pager.join("zzz.observe.jsonl"),
        "{\"kind\":\"context\",\"parse_ok\":true,\"stored_tokens\":999,\"sent_est_tokens\":900}\n",
    )
    .unwrap();
    let (o, e, c) = w.run_stdin(&["page", "report"], "");
    assert_eq!(c, 0, "report: {o}{e}");
    assert!(o.contains("session=zzz"), "{o}");
    assert!(o.contains("last_stored=999"), "{o}");
}

#[test]
fn report_prints_pager_mode_from_file() {
    let w = World::new("pmode");
    let pager = w.home.join(".caddis").join("pager");
    fs::create_dir_all(&pager).unwrap();
    fs::write(
        pager.join("s1.observe.jsonl"),
        "{\"kind\":\"context\",\"parse_ok\":true,\"stored_tokens\":1,\"sent_est_tokens\":1}\n",
    )
    .unwrap();
    let (o, e, c) = w.run_stdin(&["page", "report", "--session", "s1"], "");
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("pager_mode=observe"), "{o}");
    fs::write(pager.join("mode"), "page\n").unwrap();
    let (o, e, c) = w.run_stdin(&["page", "report", "--session", "s1"], "");
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("pager_mode=page"), "{o}");
}

#[test]
fn report_prints_pager_mode_from_session_file() {
    let w = World::new("smode");
    let pager = w.home.join(".caddis").join("pager");
    let sdir = pager.join("s1");
    fs::create_dir_all(&sdir).unwrap();
    fs::write(
        pager.join("s1.observe.jsonl"),
        "{\"kind\":\"context\",\"parse_ok\":true,\"stored_tokens\":1,\"sent_est_tokens\":1}\n",
    )
    .unwrap();
    fs::write(sdir.join("mode"), "page\n").unwrap();
    let (o, e, c) = w.run_stdin(&["page", "report", "--session", "s1"], "");
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("pager_mode=page"), "{o}");
}
