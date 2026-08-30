//! brief_lineage.rs — CARD-0264 RED-first. `caddis brief --lineage <id>`
//! must read the named lineage, and the first output line must name the
//! resolved lineage in BOTH modes so a wrong target is VISIBLE, not silent.
//!
//! RED today: `brief` ignores `--lineage` (reads default_lineage), and no
//! lineage id is printed — a successor landing on a dead lineage sees
//! "cards done: 0" with no indication which lineage was read.

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-brief-lineage-{tag}-{n}"));
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

    /// Create a lineage dir under HOME/.caddis/rotation/lines/<id>.
    fn lineage_dir(&self, id: &str) -> PathBuf {
        let d = self
            .home
            .join(".caddis")
            .join("rotation")
            .join("lines")
            .join(id);
        fs::create_dir_all(&d).unwrap();
        d
    }

    fn caddis(&self, args: &[&str]) -> (String, String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .output()
            .expect("spawn caddis");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }
}

/// RED: `caddis brief --lineage full` exits 0 and reads the NAMED lineage
/// (full state: 2 done, 1 queued), not the default. The first output line
/// names the lineage. Today the flag is ignored and no lineage is named.
#[test]
fn brief_lineage_flag_reads_named_lineage() {
    let w = World::new("flag");

    // Full lineage: 2 done cards, 1 queued.
    let full = w.lineage_dir("full");
    fs::write(
        full.join("queue"),
        "done CARD-0001\ndone CARD-0002\nCARD-0003 fix the bug\n",
    )
    .unwrap();

    // Empty lineage (so default may pick either).
    let _empty = w.lineage_dir("empty");

    let (out, err, code) = w.caddis(&["brief", "--lineage", "full"]);
    assert_eq!(
        code, 0,
        "caddis brief --lineage full must exit 0; got {code}\nstdout: {out}\nstderr: {err}"
    );

    // First output line names the lineage.
    let first_line = out.lines().next().unwrap_or("");
    assert!(
        first_line.contains("full"),
        "first line must name the lineage 'full': {first_line}\nfull output: {out}"
    );

    // The named lineage's state is visible — 2 done cards.
    assert!(
        out.contains("cards done: 2"),
        "brief --lineage full must show the full lineage state: {out}"
    );
}

/// RED: `caddis brief` (no flag) — the first output line names the
/// resolved lineage so a wrong target is VISIBLE, not silent.
#[test]
fn brief_no_flag_names_resolved_lineage() {
    let w = World::new("default");

    let full = w.lineage_dir("full");
    fs::write(
        full.join("queue"),
        "done CARD-0001\ndone CARD-0002\nCARD-0003 fix the bug\n",
    )
    .unwrap();

    let _empty = w.lineage_dir("empty");

    let (out, _err, code) = w.caddis(&["brief"]);
    assert_eq!(
        code, 0,
        "caddis brief must exit 0; got {code}\nstdout: {out}"
    );

    // First output line names the lineage (whichever was resolved).
    let first_line = out.lines().next().unwrap_or("");
    assert!(
        first_line.contains("lineage"),
        "first line must name the lineage: {first_line}\nfull output: {out}"
    );
    assert!(
        first_line.contains("full") || first_line.contains("empty"),
        "first line must contain a lineage id: {first_line}"
    );
}

/// RED: the empty lineage case is unmistakable — "cards done: 0, queued: 0"
/// is visible and the lineage name is printed so the operator knows which
/// dead lineage they landed on.
#[test]
fn brief_empty_lineage_is_unmistakable() {
    let w = World::new("empty");

    // Only an empty lineage exists.
    let _empty = w.lineage_dir("empty");

    let (out, _err, code) = w.caddis(&["brief", "--lineage", "empty"]);
    assert_eq!(
        code, 0,
        "caddis brief --lineage empty must exit 0; got {code}\nstdout: {out}"
    );

    let first_line = out.lines().next().unwrap_or("");
    assert!(
        first_line.contains("empty"),
        "first line must name the lineage 'empty': {first_line}\nfull output: {out}"
    );

    assert!(
        out.contains("cards done: 0"),
        "empty lineage must show cards done: 0: {out}"
    );
    assert!(
        out.contains("queued: 0"),
        "empty lineage must show queued: 0: {out}"
    );
}
