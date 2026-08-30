//! page mode CLI tests — CARD-0188. Hermetic HOME.
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-pmode-{tag}-{n}"));
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
        let mut child = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .spawn()
            .expect("spawn caddis");
        child.stdin.as_mut().unwrap().write_all(b"").unwrap();
        let out = child.wait_with_output().expect("caddis must finish");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }
}

#[test]
fn page_mode_set_writes_session_file() {
    let w = World::new("modeset");
    let pager = w.home.join(".caddis").join("pager");
    fs::create_dir_all(&pager).unwrap();
    fs::write(
        pager.join("s1.observe.jsonl"),
        "{\"kind\":\"context\",\"parse_ok\":true,\"stored_tokens\":1,\"sent_est_tokens\":1}\n",
    )
    .unwrap();
    let (o, e, c) = w.run(&["page", "mode", "--session", "s1", "--set", "page"]);
    assert_eq!(c, 0, "mode set: {o}{e}");
    let (o, e, c) = w.run(&["page", "report", "--session", "s1"]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("pager_mode=page"), "{o}");
}

#[test]
fn session_observe_beats_global_page() {
    let w = World::new("obsbeat");
    let pager = w.home.join(".caddis").join("pager");
    let sdir = pager.join("s1");
    fs::create_dir_all(&sdir).unwrap();
    fs::write(
        pager.join("s1.observe.jsonl"),
        "{\"kind\":\"context\",\"parse_ok\":true,\"stored_tokens\":1,\"sent_est_tokens\":1}\n",
    )
    .unwrap();
    fs::write(pager.join("mode"), "page\n").unwrap();
    fs::write(sdir.join("mode"), "observe\n").unwrap();
    let (o, e, c) = w.run(&["page", "report", "--session", "s1"]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("pager_mode=observe"), "{o}");
}
