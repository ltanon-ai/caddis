//! soul_writer.rs — CARD-0263 RED-first. The identity loop is unclosed:
//! nothing writes soul.jsonl in production. The writer belongs where
//! verdicts happen — the worker_done gate. After: DW-OK → Joy(10); 3x
//! withheld → Pain(8); compose shows the composted lesson.

use std::env;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const TEST_KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("caddis-sw-{}-{n}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn prepend_path(first: &Path) -> OsString {
    let mut out = first.as_os_str().to_os_string();
    if let Some(rest) = env::var_os("PATH") {
        out.push(if cfg!(windows) { ";" } else { ":" });
        out.push(rest);
    }
    out
}

struct World {
    home: PathBuf,
    root: PathBuf,
    herdr_fixture: PathBuf,
    warden_bin: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let herdr_fixture = root.join("herdr.json");
        fs::write(&herdr_fixture, "").unwrap();
        let warden_bin = root.join("bin");
        fs::create_dir_all(&warden_bin).unwrap();
        #[cfg(windows)]
        fs::write(
            warden_bin.join("caddis-warden.cmd"),
            "@echo off\r\nexit /b 0\r\n",
        )
        .unwrap();
        #[cfg(not(windows))]
        {
            use std::os::unix::fs::PermissionsExt;
            let p = warden_bin.join("caddis-warden");
            fs::write(&p, "#!/bin/sh\nexit 0\n").unwrap();
            fs::set_permissions(&p, fs::Permissions::from_mode(0o755)).unwrap();
        }
        Self {
            home,
            root,
            herdr_fixture,
            warden_bin,
        }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(args)
            .current_dir(&self.root)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", TEST_KEY)
            .env("CADDIS_DRAIN_HERDR", &self.herdr_fixture)
            .env("PATH", prepend_path(&self.warden_bin))
            .output()
            .expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn arm(&self) {
        let (o, e, c) = self.run(&[
            "rotate",
            "ready",
            "--kind",
            "omp",
            "--model",
            "m1",
            "--lineage",
            "line-a",
        ]);
        assert_eq!(c, 0, "ready: {o}{e}");
        let (o, e, c) = self.run(&["rotate", "arm", "--lineage", "line-a"]);
        assert_eq!(c, 0, "arm: {o}{e}");
    }

    fn queue(&self, body: &str) {
        let dir = self.line_dir();
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("queue"), body).unwrap();
    }

    fn line_dir(&self) -> PathBuf {
        self.home
            .join(".caddis")
            .join("rotation")
            .join("lines")
            .join("line-a")
    }

    fn tick(&self) -> (String, String, i32) {
        self.run(&["worker", "tick", "--lineage", "line-a"])
    }

    fn bee_script(&self) -> PathBuf {
        let s = self.root.join("bee.py");
        fs::write(&s, "import sys\nopen(sys.argv[1],'a').write('run\\n')\n").unwrap();
        s
    }

    fn compose(&self) -> (String, String, i32) {
        self.run(&["soul", "compose", "--lineage", "line-a"])
    }
}

fn read_soul(w: &World) -> String {
    fs::read_to_string(w.line_dir().join("soul.jsonl")).unwrap_or_default()
}

fn read_blockers(w: &World) -> String {
    fs::read_to_string(w.line_dir().join("blockers.jsonl")).unwrap_or_default()
}

/// DW-OK leaves one Joy entry (emotion 10, source names the card).
/// Today soul.jsonl is ABSENT — the identity loop is unclosed.
#[test]
fn done_marked_writes_joy_entry() {
    let w = World::new("dw-ok");
    w.arm();
    let bee = w.bee_script();
    let marker = w.root.join("marker.txt");
    fs::write(
        w.root.join("_card_9603.md"),
        "# Done-When\n- $ python -c pass\n",
    )
    .unwrap();
    w.queue(&format!(
        "CARD-9603 python {} {}\n",
        bee.display(),
        marker.display()
    ));
    let (o, e, c) = w.tick();
    assert_eq!(c, 0, "tick: {o}{e}");
    assert!(o.contains("DW-OK"), "done earned: {o}{e}");
    let soul = read_soul(&w);
    assert!(!soul.is_empty(), "soul.jsonl exists after DW-OK: {soul}");
    assert!(soul.contains("\"kind\":\"Joy\""), "joy entry: {soul}");
    assert!(soul.contains("CARD-9603"), "source names the card: {soul}");
    assert!(soul.contains("\"emotion\":10"), "joy lands at 10: {soul}");
    assert!(
        soul.contains("\"created_by_model\":\"caddis-worker\""),
        "worker model: {soul}"
    );
}

/// 3x withheld halts the line and writes one Pain entry whose blocker_id
/// matches the halt blocker source. Today soul.jsonl stays ABSENT.
#[test]
fn withheld_halt_writes_pain_entry() {
    let w = World::new("halt-pain");
    w.arm();
    let bee = w.bee_script();
    let marker = w.root.join("marker.txt");
    fs::write(
        w.root.join("_card_9604.md"),
        "# Withheld probe\n\n# Done-When\n\n- $ python -c \"import sys;sys.exit(1)\"\n",
    )
    .unwrap();
    w.queue(&format!(
        "CARD-9604 python {} {}\n",
        bee.display(),
        marker.display()
    ));
    let threshold = caddis_organs::watchdog::DEFAULT_MAX_FAILURES;
    for i in 1..threshold {
        let (o, e, c) = w.tick();
        assert_eq!(c, 0, "tick {i}: {o}{e}");
        assert!(o.contains("DW-FAIL"), "tick {i} withheld: {o}");
        assert!(read_soul(&w).is_empty(), "tick {i}: no soul before halt");
    }
    let (o, e, c) = w.tick();
    assert_eq!(c, 0, "halt is a line halt: {o}{e}");
    assert!(o.contains("WITHHELD-HALT"), "halt visible: {o}{e}");
    let soul = read_soul(&w);
    assert!(!soul.is_empty(), "soul.jsonl exists after halt: {soul}");
    assert!(soul.contains("\"kind\":\"Pain\""), "pain entry: {soul}");
    assert!(soul.contains("\"emotion\":8"), "pain lands at 8: {soul}");
    assert!(
        soul.contains("\"created_by_model\":\"caddis-worker\""),
        "worker model: {soul}"
    );
    let blockers = read_blockers(&w);
    let bs = "worker:CARD-9604";
    assert!(blockers.contains(bs), "halt blocker filed: {blockers}");
    assert!(
        soul.contains(&format!("\"blocker_id\":\"{bs}\"")),
        "blocker_id matches: {soul}"
    );
    assert!(soul.contains("CARD-9604"), "cause names the card: {soul}");
}

/// compose shows the lesson line once pain has composted — the identity
/// loop closed end-to-end. Pain at emotion 8 decays to 0 via CARD-0253's
/// half-life curve; the lesson survives composting.
#[test]
fn compose_shows_lesson_after_pain_composts() {
    let w = World::new("compose-lesson");
    w.arm();
    let bee = w.bee_script();
    let marker = w.root.join("marker.txt");
    fs::write(
        w.root.join("_card_9605.md"),
        "# Withheld probe\n\n# Done-When\n\n- $ python -c \"import sys;sys.exit(1)\"\n",
    )
    .unwrap();
    w.queue(&format!(
        "CARD-9605 python {} {}\n",
        bee.display(),
        marker.display()
    ));
    let threshold = caddis_organs::watchdog::DEFAULT_MAX_FAILURES;
    for _ in 0..threshold {
        let _ = w.tick();
    }
    // Age the pain entry so the decay curve fires (fresh entry has age ~0).
    // Rewrite epoch to 0 so compose sees max age: 8 -> 4 -> 2 -> 1 -> 0.
    let soul_path = w.line_dir().join("soul.jsonl");
    let raw = fs::read_to_string(&soul_path).unwrap();
    let aged: String = raw
        .lines()
        .map(|l| {
            let pat = "\"epoch\":";
            match l.find(pat) {
                None => l.to_string(),
                Some(s) => {
                    let ds = s + pat.len();
                    let de = l[ds..]
                        .find(|c: char| !c.is_ascii_digit())
                        .map(|p| ds + p)
                        .unwrap_or(l.len());
                    format!("{}\"epoch\":0{}", &l[..s], &l[de..])
                }
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(&soul_path, format!("{aged}\n")).unwrap();
    let mut composed = String::new();
    for _ in 0..8 {
        let (o, _e, _c) = w.compose();
        composed = o;
        if composed.contains("I learned") {
            break;
        }
    }
    assert!(
        composed.contains("I learned"),
        "compose shows lesson: {composed}"
    );
}
