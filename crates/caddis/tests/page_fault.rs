//! page_fault.rs — CARD-0213. Hermetic HOME. Never ~/.caddis live bag.
use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::SeqCst);
    let p = std::env::temp_dir().join(format!("caddis-pfault-{tag}-{n}"));
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
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_caddis"));
        cmd.args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("HOME", &self.home)
            .env("USERPROFILE", &self.home);
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

#[test]
fn report_prints_fault_ref_recovery_ms() {
    let w = World::new("fault");
    let pager = w.home.join(".caddis").join("pager");
    fs::create_dir_all(&pager).unwrap();
    fs::write(
        pager.join("s1.observe.jsonl"),
        "{\"kind\":\"fault\",\"recovery_ms\":150}\n{\"kind\":\"ref\"}\n",
    )
    .unwrap();
    let (o, e, c) = w.run(&["page", "report", "--session", "s1"]);
    assert_eq!(c, 0, "{o}{e}");
    assert!(o.contains("fault=1"), "{o}");
    assert!(o.contains("ref=1"), "{o}");
    assert!(o.contains("last_recovery_ms=150"), "{o}");
}
