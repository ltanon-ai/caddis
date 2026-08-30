//! tick cap tests — CARD-0199. Hermetic HOME.
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-ptick-{tag}-{n}"));
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

fn forty_spans() -> String {
    (0..40).map(|i| format!("{i},{i},400,0\n")).collect()
}

#[test]
fn default_cap_evicts_past_32() {
    let w = World::new("cap40");
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
        ],
        &forty_spans(),
    );
    assert_eq!(c, 0, "{o}{e}");
    assert_eq!(
        o.matches("evict=").count(),
        40,
        "default cap must not stop at 32: {o}"
    );
    assert!(o.contains("starved=false"), "{o}");
}

#[test]
fn explicit_cap_1_still_starves() {
    let w = World::new("cap1");
    let (o, e, c) = w.run_stdin(
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
    assert_eq!(c, 0, "{o}{e}");
    assert_eq!(o.matches("evict=").count(), 1, "cap bounds the cycle: {o}");
    assert!(o.contains("starved=true"), "{o}");
}

/// CARD-0211: era_turn dissolves stickiness before the boundary. A pinned
/// span whose turn precedes --era-turn becomes evictable.
#[test]
fn era_turn_dissolves_pinned_before_boundary() {
    let w = World::new("era5");
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
            "--era-turn",
            "5",
        ],
        "1,1,4000,1\n",
    );
    assert_eq!(c, 0, "{o}{e}");
    assert_eq!(
        o.matches("evict=").count(),
        1,
        "era dissolves pre-boundary pinned: {o}"
    );
    assert!(o.contains("starved=false"), "{o}");
}

/// CARD-0211: default era_turn 0 keeps pinned spans sticky (regression).
#[test]
fn era_turn_zero_keeps_pinned_sticky() {
    let w = World::new("era0");
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
        ],
        "1,1,4000,1\n",
    );
    assert_eq!(c, 0, "{o}{e}");
    assert_eq!(
        o.matches("evict=").count(),
        0,
        "no era -> pinned stays sticky: {o}"
    );
    assert!(o.contains("starved=true"), "{o}");
}
