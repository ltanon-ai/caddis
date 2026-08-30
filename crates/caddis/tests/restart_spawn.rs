//! restart_spawn.rs — CARD-0309. The spawn transaction's herdr contract:
//! the split is a `pane split` SUBCOMMAND call (bare `herdr --current …`
//! options exit 2 with empty stdout on the estate's .cmd shim), and a
//! nonzero herdr exit is a failure, never an empty success.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);
const KEY: &str = "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-spawn-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p); // swallow: best-effort-cleanup — stale temp dir from a prior run
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    home: PathBuf,
    line: PathBuf,
}

impl World {
    fn new(tag: &str) -> (Self, PathBuf) {
        let root = tmp(tag);
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let w = Self {
            line: home.join(".caddis/rotation/lines/lin-s"),
            home,
        };
        (w, root)
    }

    fn run(
        &self,
        args: &[&str],
        envs: &[(&str, &str)],
        cwd: Option<&Path>,
    ) -> (String, String, i32) {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis"));
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home)
            .env("CADDIS_HMAC_KEY", KEY)
            .env("CADDIS_SKIP_WARDEN", "1")
            .env_remove("HERDR_PANE_ID")
            .env_remove("CADDIS_HERDR_BIN");
        for (k, v) in envs {
            cmd.env(k, v);
        }
        if let Some(dir) = cwd {
            cmd.current_dir(dir);
        }
        let out = cmd.output().expect("caddis must spawn");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    /// Seed the lineage the way the organs do: ready AT the work root
    /// (CARD-0303 stamps it) + arm, with a kind/model.
    fn seed_as(&self, root: &Path, kind: &str, model: &str) {
        let (o, e, c) = self.run(
            &[
                "rotate",
                "ready",
                "--kind",
                kind,
                "--model",
                model,
                "--lineage",
                "lin-s",
            ],
            &[],
            Some(root),
        );
        assert_eq!(c, 0, "ready: {o}{e}");
        assert!(
            self.line.join("ready.root").is_file(),
            "ready.root missing: {o}"
        );
        let (o, e, c) = self.run(&["rotate", "arm", "--lineage", "lin-s"], &[], None);
        assert_eq!(c, 0, "arm: {o}{e}");
    }

    fn seed(&self, root: &Path) {
        self.seed_as(root, "omp", "m1");
    }
}

/// A fake herdr .cmd shim: appends its argv to argv.log, answers only a
/// `pane split` call with a pane_id-bearing response (E2 shape).
#[cfg(windows)]
fn write_fake_herdr(root: &Path) -> (PathBuf, PathBuf) {
    let bin = root.join("fake-herdr.cmd");
    let log = root.join("argv.log");
    let body = format!(
        "@echo off\r\necho %*>> \"{}\"\r\nif \"%1\"==\"pane\" if \"%2\"==\"split\" echo {{\"result\":{{\"pane\":{{\"pane_id\":\"wS:p9\"}}}}}}\r\n",
        log.display()
    );
    fs::write(&bin, body).unwrap();
    (bin, log)
}

/// CARD-0309: spawn's split must be a `pane split` subcommand call — the
/// live 2026-08-29 rotation proved bare options split nothing.
#[cfg(windows)]
#[test]
fn spawn_split_uses_the_pane_split_subcommand() {
    let (world, root) = World::new("shape");
    world.seed(&root);
    let (bin, log) = write_fake_herdr(&root);
    let (o, e, c) = world.run(
        &["restart", "spawn", "--lineage", "lin-s"],
        &[("CADDIS_HERDR_BIN", bin.to_str().unwrap())],
        None,
    );
    assert_eq!(c, 0, "spawn: {o}{e}");
    assert!(o.contains("pane: wS:p9"), "pane id not adopted: {o}");
    assert!(o.contains("seat: sent"), "seat not booted: {o}");
    let argv = fs::read_to_string(&log).unwrap();
    let first = argv.lines().next().unwrap_or("(no call logged)");
    assert!(
        first.starts_with("pane split --current"),
        "first herdr call was not a pane split: {first}"
    );
}

/// CARD-0309: a herdr that exits nonzero with empty stdout must fail
/// spawn as unreachable — never `pane split returned no id: ` (empty).
/// Portable: caddis itself answers an unknown `pane` subcommand with a
/// nonzero exit and empty stdout.
#[test]
fn spawn_fails_closed_when_herdr_exits_nonzero() {
    let (world, root) = World::new("nonzero");
    world.seed(&root);
    let (o, e, c) = world.run(
        &["restart", "spawn", "--lineage", "lin-s"],
        &[("CADDIS_HERDR_BIN", env!("CARGO_BIN_EXE_caddis"))],
        None,
    );
    assert_ne!(c, 0, "spawn must fail: {o}");
    assert!(
        e.contains("herdr unreachable"),
        "failed herdr call must read as unreachable, stderr: {e}"
    );
}

/// CARD-0315: the `pane run` payload must BOOT a seat (agent CLI +
/// model + pointer), not send a bare pointer into a dead shell.
#[cfg(windows)]
#[test]
fn spawn_boots_the_seat_not_a_bare_pointer() {
    let (world, root) = World::new("seat-omp");
    world.seed(&root);
    let (bin, log) = write_fake_herdr(&root);
    let (o, e, c) = world.run(
        &["restart", "spawn", "--lineage", "lin-s"],
        &[("CADDIS_HERDR_BIN", bin.to_str().unwrap())],
        None,
    );
    assert_eq!(c, 0, "spawn: {o}{e}");
    let argv = fs::read_to_string(&log).unwrap();
    let run_line = argv
        .lines()
        .find(|l| l.starts_with("pane run"))
        .unwrap_or("(no pane run call)");
    assert!(
        run_line.contains("omp --model m1"),
        "seat boot missing omp launch: {run_line}"
    );
    assert!(
        run_line.contains("'caddis restart enter --lineage lin-s'"),
        "pointer must ride inside the seat prompt: {run_line}"
    );

    let (w2, r2) = World::new("seat-claude");
    w2.seed_as(&r2, "claude", "m9");
    let (bin2, log2) = write_fake_herdr(&r2);
    let (o2, e2, c2) = w2.run(
        &["restart", "spawn", "--lineage", "lin-s"],
        &[("CADDIS_HERDR_BIN", bin2.to_str().unwrap())],
        None,
    );
    assert_eq!(c2, 0, "spawn: {o2}{e2}");
    let argv2 = fs::read_to_string(&log2).unwrap();
    let run2 = argv2
        .lines()
        .find(|l| l.starts_with("pane run"))
        .unwrap_or("(no pane run call)");
    assert!(
        run2.contains("claude --model m9"),
        "claude seat must launch claude: {run2}"
    );
}

/// CARD-0315: the predecessor's heartbeat must NOT mask an unbooted
/// successor — only the split pane's own heartbeat suppresses the
/// armed-never-woke marker.
#[cfg(windows)]
#[test]
fn spawn_marks_armed_never_woke_on_foreign_heartbeat() {
    let (world, root) = World::new("foreign-hb");
    world.seed(&root);
    fs::write(world.line.join("heartbeat.receipt"), "pane=w1:pOLD\nts=1\n").unwrap();
    let (bin, _log) = write_fake_herdr(&root);
    let (o, e, c) = world.run(
        &["restart", "spawn", "--lineage", "lin-s"],
        &[("CADDIS_HERDR_BIN", bin.to_str().unwrap())],
        None,
    );
    assert_eq!(c, 0, "spawn: {o}{e}");
    assert!(
        world.line.join("armed-never-woke.lease").is_file(),
        "a foreign heartbeat must not suppress the wake marker"
    );
}

/// CARD-0315 guard: the split pane's own heartbeat IS the wake.
#[cfg(windows)]
#[test]
fn spawn_accepts_its_own_heartbeat() {
    let (world, root) = World::new("own-hb");
    world.seed(&root);
    fs::write(world.line.join("heartbeat.receipt"), "pane=wS:p9\nts=1\n").unwrap();
    let (bin, _log) = write_fake_herdr(&root);
    let (o, e, c) = world.run(
        &["restart", "spawn", "--lineage", "lin-s"],
        &[("CADDIS_HERDR_BIN", bin.to_str().unwrap())],
        None,
    );
    assert_eq!(c, 0, "spawn: {o}{e}");
    assert!(
        !world.line.join("armed-never-woke.lease").is_file(),
        "the split pane's heartbeat is the wake"
    );
    assert!(o.contains("heartbeat: present"), "{o}");
}
