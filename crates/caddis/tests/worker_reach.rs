//! worker_reach.rs tests — CARD-0327. LAYER 3: a dispatch that lands
//! DONE with a callerless created unit becomes a talk finding the
//! retire-gate can hold. Hermetic HOME; the live bag is never touched.

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
    let p = std::env::temp_dir().join(format!("caddis-wr-{}-{n}-{tag}", std::process::id()));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

fn prepend_path(first: &Path) -> OsString {
    let mut out = first.as_os_str().to_os_string();
    if let Some(rest) = env::var_os("PATH") {
        out.push(";");
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
            .env("PATH", prepend_path(&self.warden_bin))
            .env("CADDIS_DRAIN_HERDR", &self.herdr_fixture)
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

    fn write_card(&self, body: &str) {
        fs::write(self.root.join("_card_0327.md"), body).unwrap();
    }

    fn turns(&self) -> String {
        fs::read_to_string(self.line_dir().join("talk/turns.jsonl")).unwrap_or_default()
    }
}

const LONELY_CARD: &str = "---\nid: CARD-0327\nclass: feat\nowner: t\n---\n\
# lonely unit probe\n\n# Done-When\n- $ python -c pass\n\n# RED-TEST\nit failed before\n\n\
# EXECUTION\n\n```yaml\nlevel: L1\nblast: 1\nclaims-forbidden: true\nanchors:\n  \
- path: crates/mini/src/lonely.rs\n    content: |\n      filler\nallowlist:\n  \
- create crates/mini/src/lonely.rs\n```\n";

/// A DONE dispatch whose created unit has no repo caller -> a talk
/// finding lands, naming the unit (the retire-gate can hold the line).
#[test]
fn lonely_unit_lands_as_a_finding() {
    let w = World::new("lonely");
    w.arm();
    w.write_card(LONELY_CARD);
    w.queue("CARD-0327 python -c pass\n");
    let (o, e, c) = w.run(&["worker", "tick", "--lineage", "line-a"]);
    assert_eq!(c, 0, "tick: {o}{e}");
    assert!(o.contains("DW-OK"), "done earned: {o}{e}");
    let turns = w.turns();
    assert!(
        turns.contains("\"kind\":\"finding\""),
        "a finding was posted: {turns}"
    );
    assert!(
        turns.contains("lonely"),
        "the finding names the unit: {turns}"
    );
    assert!(
        turns.contains("CARD-0327"),
        "the finding names the card: {turns}"
    );
}

/// A created unit another source file mentions -> silence (no finding).
#[test]
fn wired_unit_posts_nothing() {
    let w = World::new("wired");
    w.arm();
    w.write_card(&LONELY_CARD.replace("lonely", "hosted"));
    fs::create_dir_all(w.root.join("crates/mini/src")).unwrap();
    fs::write(
        w.root.join("crates/mini/src/main.rs"),
        "mod hosted;\nfn main() { hosted::go(); }\n",
    )
    .unwrap();
    w.queue("CARD-0327 python -c pass\n");
    let (o, e, c) = w.run(&["worker", "tick", "--lineage", "line-a"]);
    assert_eq!(c, 0, "tick: {o}{e}");
    assert!(o.contains("DW-OK"), "done earned: {o}{e}");
    assert!(
        !w.turns().contains("\"kind\":\"finding\""),
        "wired unit stays silent"
    );
}

/// A card with no created compiled units -> no reach noise at all.
#[test]
fn no_creates_no_finding() {
    let w = World::new("plain");
    w.arm();
    w.write_card(
        "---\nid: CARD-0327\nclass: docs\nowner: t\n---\n\
# plain\n\n# Done-When\n- $ python -c pass\n\n# RED-TEST\nit failed before\n",
    );
    w.queue("CARD-0327 python -c pass\n");
    let (o, e, c) = w.run(&["worker", "tick", "--lineage", "line-a"]);
    assert_eq!(c, 0, "tick: {o}{e}");
    assert!(o.contains("DW-OK"), "done earned: {o}{e}");
    assert!(
        !w.turns().contains("\"kind\":\"finding\""),
        "no creates, no noise"
    );
}
