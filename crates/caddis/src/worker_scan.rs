//! worker_scan.rs — CARD-0219. The scanner organ: mechanical quality
//! gate over the whole workspace. fmt + clippy -D warnings + tests +
//! AGENT-LAW line census. Appends scan.log; the board shows the last
//! verdict. Hermetic tests override the suite via CADDIS_SCAN_SUITE
//! and the census root via CADDIS_SCAN_ROOT (same pattern as drain
//! fixtures). Scan never spawns chairs.

use std::env;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::lineage;

pub enum Error {
    Usage(String),
    Fail(String),
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Error::Usage(s) | Error::Fail(s) => write!(f, "{s}"),
        }
    }
}

const CENSUS_MAX: usize = 280;

fn real_suite() -> Vec<(&'static str, Vec<&'static str>)> {
    vec![
        ("fmt", vec!["cargo", "fmt", "--check"]),
        (
            "clippy",
            vec![
                "cargo",
                "clippy",
                "--workspace",
                "--all-targets",
                "--",
                "-D",
                "warnings",
            ],
        ),
        ("test", vec!["cargo", "test", "--workspace"]),
    ]
}

fn fixture_suite(path: &Path) -> Option<Vec<(String, Vec<String>)>> {
    let text = fs::read_to_string(path).ok()?;
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let mut parts = line.split_whitespace();
        let name = parts.next()?.to_string();
        let argv: Vec<String> = parts.map(str::to_string).collect();
        out.push((name, argv));
    }
    Some(out)
}

pub fn run(args: &[String]) -> Result<(), Error> {
    let (id, rest) = lineage::take(args).map_err(Error::Usage)?;
    if let Some(a) = rest.first() {
        return Err(Error::Usage(format!("unknown argument {a}")));
    }
    let dir = lineage::dir(&id).map_err(Error::Fail)?;
    let suite = resolve_suite();
    let mut results = Vec::new();
    for (name, argv) in suite {
        live(&dir, &name, "start");
        let ok = run_cmd(&argv);
        live(&dir, &name, if ok { "pass" } else { "fail" });
        results.push((name, ok));
    }
    let (census_ok, worst) = census();
    results.push(("census".into(), census_ok));
    report(&dir, &results, worst.as_deref())
}

/// Fixture suite when CADDIS_SCAN_SUITE names a readable file, else the
/// real workspace suite. One list either way — the run loop never forks.
fn resolve_suite() -> Vec<(String, Vec<String>)> {
    let fixture = env::var_os("CADDIS_SCAN_SUITE").map(PathBuf::from);
    if let Some(suite) = fixture.as_deref().and_then(fixture_suite) {
        return suite;
    }
    real_suite()
        .into_iter()
        .map(|(name, argv)| {
            (
                name.to_string(),
                argv.iter().map(|s| s.to_string()).collect(),
            )
        })
        .collect()
}

fn report(dir: &Path, results: &[(String, bool)], worst: Option<&str>) -> Result<(), Error> {
    let mut all = true;
    for (name, ok) in results {
        println!("{name:<8} {}", if *ok { "pass" } else { "FAIL" });
        all = all && *ok;
    }
    if let Some(w) = worst {
        println!("census-worst {w}");
    }
    println!("SUMMARY {}", if all { "pass" } else { "fail" });
    append_log(dir, results, worst);
    if all {
        Ok(())
    } else {
        Err(Error::Fail("scan fail".into()))
    }
}

fn run_cmd(argv: &[String]) -> bool {
    Command::new(&argv[0])
        .args(&argv[1..])
        .status()
        .map(|s| s.code().unwrap_or(1) == 0)
        .unwrap_or(false)
}

/// AGENT-LAW: every .rs under the root (crates/ in production) is
/// <=280 lines. Returns (ok, worst "file:lines").
fn census() -> (bool, Option<String>) {
    let root = env::var_os("CADDIS_SCAN_ROOT")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("crates"));
    let mut worst: Option<(usize, String)> = None;
    let mut ok = true;
    let mut stack = vec![root];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for ent in entries.flatten() {
            let p = ent.path();
            if p.is_dir() {
                stack.push(p);
                continue;
            }
            if p.extension().and_then(|e| e.to_str()) != Some("rs") {
                continue;
            }
            let Ok(text) = fs::read_to_string(&p) else {
                continue;
            };
            let n = text.lines().count();
            if n > CENSUS_MAX {
                ok = false;
            }
            let worse = worst.as_ref().map(|(w, _)| n > *w).unwrap_or(true);
            if worse {
                worst = Some((n, p.display().to_string()));
            }
        }
    }
    (ok, worst.map(|(n, p)| format!("{p}:{n}")))
}

fn live(dir: &Path, check: &str, state: &str) {
    use std::io::Write;
    let ts = crate::receipt::timestamp();
    let line = format!("{{\"check\":\"{check}\",\"state\":\"{state}\",\"ts\":\"{ts}\"}}\n");
    // swallow: best-effort-telemetry
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("scan.live"))
    {
        let _ = writeln!(f, "{line}"); // swallow: best-effort-telemetry
    }
}

fn append_log(dir: &Path, results: &[(String, bool)], worst: Option<&str>) {
    let _ = fs::create_dir_all(dir); // swallow: checked-elsewhere
    let ts = crate::receipt::timestamp();
    let mut line = String::from("{\"kind\":\"scan\"");
    for (name, ok) in results {
        line.push_str(&format!(
            ",\"{}\":\"{}\"",
            name,
            if *ok { "pass" } else { "fail" }
        ));
    }
    if let Some(w) = worst {
        line.push_str(&format!(",\"worst\":\"{w}\""));
    }
    line.push_str(&format!(",\"ts\":\"{ts}\"}}"));
    // swallow: best-effort-telemetry
    if let Ok(mut f) = OpenOptions::new()
        .create(true)
        .append(true)
        .open(dir.join("scan.log"))
    {
        let _ = writeln!(f, "{line}"); // swallow: best-effort-telemetry
    }
}
