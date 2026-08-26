//! live_remember.rs — slice (b) LIVE gates against the real machine
//! (quorum verdict 2026-08-26, gates (b)→(c) and (c)→live).
//!
//! ```text
//! cargo test -p caddis-memory --test live_remember -- --ignored --nocapture
//! ```
//!
//! What is REAL here:
//! - the real registry file (`~/.config/caddis/collections.json`) — the
//!   (b)→(c) "registry bootstrap succeeds on real config" leg;
//! - the real warden binary in frame mode (no args, stdin frame), which
//!   APPENDS to the real ledger (`~/.caddis/warden-ledger.jsonl`) — allow
//!   verdicts record, by the warden's own law;
//! - the real qmd engine, over a THROWAWAY project-local index in a temp
//!   dir, for the BM25 search-hit leg.
//!
//! What is NOT touched: the machine-global qmd index — `qmd collection
//! add` for the two targets is a deploy step AFTER this probe (verdict,
//! gate (c)→live). The one memory this probe writes into
//! `~/.omp/sergeant/state/memory/` is a REAL memory and stays: it is the
//! probe's own honest record.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use caddis_memory::bootstrap::{bootstrap, sergeant_state_dir, SERGEANT_STATE};
use caddis_memory::exec::{Job, RealRunner, Runner};
use caddis_memory::recall::MemoryConfig;
use caddis_memory::registry::Registry;
use caddis_memory::remember::MemoryDoc;
use caddis_memory::writer::{remember, RememberConfig};

fn warden_launcher() -> Vec<String> {
    if let Ok(bin) = std::env::var("CADDIS_WARDEN_BIN") {
        if !bin.trim().is_empty() {
            return vec![bin];
        }
    }
    vec!["E:/ClaudeToolbox/caddis/target/release/caddis-warden.exe".into()]
}

fn ledger_path() -> PathBuf {
    if let Ok(p) = std::env::var("CADDIS_WARDEN_LEDGER") {
        if !p.is_empty() {
            return PathBuf::from(p);
        }
    }
    let home = std::env::var("USERPROFILE")
        .or_else(|_| std::env::var("HOME"))
        .expect("home resolves");
    Path::new(&home).join(".caddis").join("warden-ledger.jsonl")
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs()
}

fn qmd(runner: &mut RealRunner, workdir: &Path, args: &[&str]) -> String {
    let launcher = MemoryConfig::detect().launcher;
    let job = Job {
        launcher,
        args: args.iter().map(|s| s.to_string()).collect(),
        workdir: Some(workdir.to_path_buf()),
        timeout: Duration::from_secs(120),
        stdin_data: None,
    };
    let out = runner.run(&job);
    assert!(!out.timed_out, "qmd {args:?} timed out");
    assert_eq!(out.code, Some(0), "qmd {args:?} failed: {}", out.stderr.trim());
    out.stdout
}

/// (b)→(c) gate leg: bootstrap the two Q3 write targets into the REAL
/// registry at the Q6 default home, idempotently.
#[test]
#[ignore = "live: writes the organ registry file at the Q6 default home (real config)"]
fn live_bootstrap_real_config() {
    let state_dir = sergeant_state_dir().expect("home resolves");
    let path = Registry::default_path();
    let mut reg = Registry::load(&path).expect("real registry loads or is first-run empty");

    let rep = bootstrap(&mut reg, &state_dir).expect("bootstrap on real config");
    println!(
        "bootstrap @ {}: created={:?} upserted={:?} (entries={})",
        path.display(),
        rep.created_dirs,
        rep.upserted,
        reg.entries().len()
    );

    let mut reloaded = Registry::load(&path).expect("reload after save");
    for name in ["sergeant-state", "sergeant-briefs"] {
        let e = reloaded.get(name);
        assert_eq!(e.owner, "sergeant", "{name} owned by the organ");
        assert!(!e.public, "{name} private (Q6)");
        let root = e.root.expect("{name} carries its sandbox root");
        assert!(root.is_dir(), "{name} root exists: {}", root.display());
    }
    reloaded.validate_roots().expect("real roots pairwise non-overlapping");

    let again = bootstrap(&mut reloaded, &state_dir).expect("second bootstrap");
    assert!(again.created_dirs.is_empty() && again.upserted.is_empty(), "idempotent on real config");
}

/// (c)→live gate: ONE real memory through the whole ratified flow —
/// registry-derived sandbox, real warden verdict, real ledger row, on-disk
/// stamp audit, active_heads view, and a BM25 hit over the real qmd engine.
#[test]
#[ignore = "live: real warden (appends to the real ledger) + real qmd engine + one real memory file"]
fn live_one_memory_probe() {
    let state_dir = sergeant_state_dir().expect("home resolves");
    let root = state_dir.join("memory");
    let mut reg = Registry::load(&Registry::default_path()).expect("registry");
    let rep = bootstrap(&mut reg, &state_dir).expect("bootstrap");
    println!("bootstrap: created={:?} upserted={:?}", rep.created_dirs, rep.upserted);

    let cfg = RememberConfig::from_registry(
        &reg,
        warden_launcher(),
        Duration::from_secs(30),
        Duration::from_secs(60),
    );

    // A nonce makes the BM25 hit provably THIS doc, not a neighbor.
    let ts = now();
    let nonce = format!("caddisprobe{ts}x");
    let mut front: BTreeMap<String, String> = BTreeMap::new();
    front.insert("title".into(), format!("caddis-remember live probe {ts}"));
    front.insert("kind".into(), "probe".into());
    front.insert("source".into(), "caddis-memory tests/live_remember".into());
    let doc = MemoryDoc {
        front,
        body: format!(
            "First live write through the P3 remember() flow (slice b probe, {ts}Z epoch). \
             This memory IS the probe: warden-gated, ledger-recorded, sandboxed to the \
             registered sergeant-state root. Nonce {nonce}."
        ),
    };
    let draft = doc.render();

    let mut runner = RealRunner;
    let r = remember(&mut runner, &cfg, doc, &root, ts).expect("warden-gated write lands");
    println!("remembered: {} seq={} tx={}", r.path.display(), r.seq, r.tx_hash);
    assert!(r.seq > 0, "verdict carried a ledger seq");
    assert!(r.tx_hash.starts_with("wardn"), "I4 stamp is a ledger row id");

    // --- ledger row: the stamp points at a REAL row in the real ledger ---
    // The warden flushes its append asynchronously: the verdict (and its tx
    // row id) returns before the row is durable in the file. Poll briefly —
    // the row must APPEAR; a pointer to nothing is an I4 violation.
    let tx = r.tx_hash.as_str();
    let mut ledger = String::new();
    for _ in 0..40 {
        ledger = fs::read_to_string(ledger_path()).expect("real ledger readable");
        if ledger.contains(tx) {
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    assert!(
        ledger.contains(tx),
        "ledger row {tx} exists in the real ledger (I4 = pointer, not self-hash)"
    );

    // --- stamp audit (I1): file minus the two stamp lines == the draft ---
    let on_disk = fs::read_to_string(&r.path).unwrap();
    let stripped: String = on_disk
        .lines()
        .filter(|l| !(l.starts_with("warden_seq: ") || l.starts_with("warden_tx_hash: ")))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    assert_eq!(stripped, draft, "on-disk doc == warden-judged draft + exactly the two stamps");

    // --- active_heads view: the new doc is the recorded tip ---
    let heads = fs::read_to_string(root.join("active_heads.json")).unwrap();
    let docid = r.path.file_stem().unwrap().to_string_lossy().to_string();
    assert!(heads.contains(&docid), "heads view carries {docid}");

    // --- BM25 hit over the real engine, throwaway project-local index ---
    let td = std::env::temp_dir().join(format!("caddis-live-bm25-{}", std::process::id()));
    let _ = fs::remove_dir_all(&td);
    fs::create_dir_all(&td).unwrap();
    let fwd_root = root.to_string_lossy().replace('\\', "/");
    let mut bm = RealRunner;
    qmd(&mut bm, &td, &["init"]);
    qmd(&mut bm, &td, &["collection", "add", SERGEANT_STATE, &fwd_root]);
    qmd(&mut bm, &td, &["update"]);
    let hits = qmd(&mut bm, &td, &["search", &nonce, "--json"]);
    println!("bm25 hits ({}): {}", nonce, hits.trim());
    assert!(
        hits.contains(&nonce) && hits.contains(&docid),
        "BM25 search finds the new memory by its unique nonce"
    );
    let _ = fs::remove_dir_all(&td); // throwaway index gone; global index untouched

    println!(
        "LIVE PROBE OK: ledger row + stamp audit + heads view + BM25 hit all proved against the real machine"
    );
}
