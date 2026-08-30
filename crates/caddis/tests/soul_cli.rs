//! soul_cli.rs — CARD-0255 RED-first. Wires the soul organ (CARD-0253)
//! into the kernel: `caddis soul compose` and `caddis brief` identity HEAD.
//!
//! RED today: `caddis soul compose` exits with a usage error — no `soul`
//! subcommand exists; `caddis brief` prints no archetype and no "I learned"
//! line. After: `soul compose` exits 0, stdout carries the archetype line AND
//! a `- I learned` line AND the joy line; blockers.jsonl gains a
//! `soul-reminder:b-900` row; `brief` contains BOTH the identity block and
//! `cards done:`.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-soul-cli-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    home: PathBuf,
    line_dir: PathBuf,
}

impl World {
    fn new(tag: &str, lineage: &str) -> Self {
        let root = tmp(tag);
        let home = root.join("home");
        let line_dir = home
            .join(".caddis")
            .join("rotation")
            .join("lines")
            .join(lineage);
        fs::create_dir_all(&line_dir).unwrap();
        Self { home, line_dir }
    }

    fn run(&self, args: &[&str]) -> (String, String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(args)
            .stdin(Stdio::null())
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

    /// Seed the lineage's soul.jsonl and blockers.jsonl per the card RED spec:
    /// one Pain{cause, blocker_id:"b-900", emotion:1, epoch:0}, one Joy, one
    /// open blocker b-900.
    fn seed_soul_fixture(&self) {
        let soul = self.line_dir.join("soul.jsonl");
        let blockers = self.line_dir.join("blockers.jsonl");

        // Pain: cause "open pain", blocker_id "b-900", emotion 1, epoch 0.
        let pain = "{\"kind\":\"Pain\",\"cause\":\"open pain\",\"blocker_id\":\"b-900\",\
                   \"lesson\":\"caution\",\"emotion\":1,\"epoch\":0,\"created_by_model\":\"m\"}\n";
        fs::write(&soul, pain).unwrap();

        // Joy: source "green pipeline", emotion 10, epoch 0.
        let joy = "{\"kind\":\"Joy\",\"source\":\"green pipeline\",\"lesson\":\"\",\
                  \"emotion\":10,\"epoch\":0,\"created_by_model\":\"m\"}\n";
        let mut f = fs::OpenOptions::new().append(true).open(&soul).unwrap();
        f.write_all(joy.as_bytes()).unwrap();

        // Open blocker b-900.
        let blocker = "{\"source\":\"b-900\",\"reason\":\"broken build\",\
                      \"ts\":\"2026-08-28T00:00:00Z\"}\n";
        fs::write(&blockers, blocker).unwrap();
    }
}

/// RED: `caddis soul compose --lineage <id>` exits 0; stdout contains the
/// archetype line (default fail-soft one-liner), a `- I learned` line, and the
/// joy line; the compost pass files a `soul-reminder:b-900` blocker.
#[test]
fn soul_compose_emits_identity_and_files_reminder() {
    let w = World::new("compose", "overnight");
    w.seed_soul_fixture();

    // No archetype.md → fail-soft default one-liner.
    let (out, err, code) = w.run(&["soul", "compose", "--lineage", "overnight"]);
    assert_eq!(code, 0, "soul compose must exit 0: {out}{err}");

    // Archetype default line present.
    assert!(
        out.contains("ARCHETYPE: unnamed careful builder."),
        "default archetype: {out}"
    );
    // Pain composted (emotion 1, age > max_age 3 → 1/2 = 0 → Lesson) → "I learned".
    assert!(out.contains("- I learned"), "lesson line: {out}");
    // Joy persists.
    assert!(out.contains("joy from green pipeline"), "joy line: {out}");

    // Compost filed a reminder for the open blocker b-900.
    let blockers = fs::read_to_string(w.line_dir.join("blockers.jsonl")).unwrap();
    assert!(
        blockers.contains("soul-reminder:b-900"),
        "reminder filed: {blockers}"
    );
}

/// RED: `caddis brief` output contains BOTH the identity block (from soul
/// compose) and the existing `cards done:` state line.
#[test]
fn brief_carries_identity_head_and_state() {
    let w = World::new("brief", "overnight");
    w.seed_soul_fixture();

    let (out, err, code) = w.run(&["brief"]);
    assert_eq!(code, 0, "brief must exit 0: {out}{err}");

    // Identity block (default archetype) above the state line.
    assert!(
        out.contains("ARCHETYPE: unnamed careful builder."),
        "identity head: {out}"
    );
    // Existing state line still present.
    assert!(out.contains("cards done:"), "state line: {out}");
    // The identity block sits ABOVE the state line, separated by a blank line.
    let head_idx = out.find("ARCHETYPE:").unwrap();
    let state_idx = out.find("cards done:").unwrap();
    assert!(head_idx < state_idx, "identity before state: {out}");
}
