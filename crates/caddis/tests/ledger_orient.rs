//! ledger_orient.rs — CARD-LEDGER-DB-3. Hermetic. Never ~/.caddis live bag.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{SystemTime, UNIX_EPOCH};

fn bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_caddis"))
}

fn python() -> Command {
    for bin in ["python", "python3"] {
        let mut c = Command::new(bin);
        c.arg("-c")
            .arg("import sqlite3")
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if c.status().map(|s| s.success()).unwrap_or(false) {
            return Command::new(bin);
        }
    }
    let mut out = Command::new("py");
    out.arg("-3");
    out
}

fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64
}

fn insert_json(db: &Path, py_tool: &Path, row: &str) {
    let mut cmd = python();
    let mut child = cmd
        .arg(py_tool)
        .arg("insert")
        .arg("--db")
        .arg(db)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn python insert");
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(row.as_bytes())
        .unwrap();
    let out = child.wait_with_output().unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
}

fn seed(db: &Path, py_tool: &Path) {
    let now = unix_now().to_string();
    let rows = [
        format!(
            r#"{{"ts":"{now}","project":"caddis-workshop","from":"omp","type":"tool.read","verdict":"allow","body":"allow|read|C:/x/caddis-workshop/a.rs||","path":"C:/x/caddis-workshop/a.rs"}}"#
        ),
        format!(
            r#"{{"ts":"{now}","project":"caddis-workshop","from":"omp","type":"tool.write","verdict":"deny","body":"deny|write|C:/x/caddis-workshop/b.rs|caddis-warden [size.file]: too big|","path":"C:/x/caddis-workshop/b.rs"}}"#
        ),
        format!(
            r#"{{"ts":"{now}","project":"caddis-workshop","from":"omp","type":"tool.write","verdict":"steer","body":"steer|write|C:/x/caddis-workshop/c.rs|caddis-warden [nobs]: skip|","path":"C:/x/caddis-workshop/c.rs"}}"#
        ),
        format!(
            r#"{{"ts":"{now}","project":"other","from":"omp","type":"tool.read","verdict":"allow","body":"allow|read|C:/x/other/z.rs||","path":"C:/x/other/z.rs"}}"#
        ),
    ];
    for row in rows {
        insert_json(db, py_tool, &row);
    }
}

fn count_rows(db: &Path) -> i64 {
    let mut cmd = python();
    let code = format!(
        "import sqlite3; print(sqlite3.connect(r'{}').execute('select count(*) from verdicts').fetchone()[0])",
        db.display()
    );
    cmd.arg("-c").arg(code);
    let out = cmd.output().expect("count");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).trim().parse().unwrap()
}

#[test]
fn orient_prints_packet_not_the_bag() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("caddis-orient-{stamp}"));
    fs::create_dir_all(&dir).unwrap();
    let db = dir.join("ledger.sqlite");
    let tool = dir.join("ledger_tool.py");
    let py = include_str!("../../../tools/ledger_sqlite.py");
    fs::write(&tool, py).unwrap();
    seed(&db, &tool);

    let out = Command::new(bin())
        .args(["ledger", "orient", "--project", "caddis-workshop"])
        .env("CADDIS_WARDEN_LEDGER_SQLITE", &db)
        .output()
        .expect("run caddis ledger orient");
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        out.status.success(),
        "status={} stdout={stdout} stderr={stderr}",
        out.status
    );
    let lines = stdout.lines().count();
    assert!(lines < 80, "orient dumped the bag: {lines} lines\n{stdout}");
    assert!(stdout.contains("PROJECT caddis-workshop"), "{stdout}");
    assert!(stdout.contains("allow="), "{stdout}");
    assert!(stdout.contains("deny="), "{stdout}");
    assert!(stdout.contains("steer="), "{stdout}");
    assert!(stdout.contains("last_deny:"), "{stdout}");
    assert!(stdout.contains("last_laws:"), "{stdout}");
    assert!(stdout.contains("last_20:"), "{stdout}");
    assert!(stdout.contains("caddis-warden [size.file]"), "{stdout}");
    assert!(!stdout.contains("C:/x/other/z.rs"), "{stdout}");
}

#[test]
fn orient_default_window_hides_old_rows() {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("caddis-orient-win-{stamp}"));
    fs::create_dir_all(&dir).unwrap();
    let db = dir.join("ledger.sqlite");
    let tool = dir.join("ledger_tool.py");
    fs::write(&tool, include_str!("../../../tools/ledger_sqlite.py")).unwrap();
    let now = unix_now();
    let old = now - 91 * 86400;
    insert_json(
        &db,
        &tool,
        &format!(
            r#"{{"ts":"{now}","project":"caddis-workshop","from":"omp","type":"tool.read","verdict":"allow","body":"allow|read|C:/x/now.rs||","path":"C:/x/now.rs"}}"#
        ),
    );
    insert_json(
        &db,
        &tool,
        &format!(
            r#"{{"ts":"{old}","project":"caddis-workshop","from":"omp","type":"tool.write","verdict":"deny","body":"deny|write|C:/x/old.rs|old deny|","path":"C:/x/old.rs"}}"#
        ),
    );
    assert_eq!(count_rows(&db), 2);

    let def = Command::new(bin())
        .args(["ledger", "orient", "--project", "caddis-workshop"])
        .env("CADDIS_WARDEN_LEDGER_SQLITE", &db)
        .output()
        .expect("default orient");
    let def_out = String::from_utf8_lossy(&def.stdout);
    assert!(
        def.status.success(),
        "{def_out} {}",
        String::from_utf8_lossy(&def.stderr)
    );
    assert!(def_out.contains("C:/x/now.rs"), "{def_out}");
    assert!(
        !def_out.contains("C:/x/old.rs"),
        "default window leaked old row:\n{def_out}"
    );

    let wide = Command::new(bin())
        .args([
            "ledger",
            "orient",
            "--project",
            "caddis-workshop",
            "--since",
            "0",
        ])
        .env("CADDIS_WARDEN_LEDGER_SQLITE", &db)
        .output()
        .expect("since 0 orient");
    let wide_out = String::from_utf8_lossy(&wide.stdout);
    assert!(
        wide.status.success(),
        "{wide_out} {}",
        String::from_utf8_lossy(&wide.stderr)
    );
    assert!(
        wide_out.contains("C:/x/old.rs"),
        " --since 0 missing old row:\n{wide_out}"
    );
    assert_eq!(count_rows(&db), 2, "orient deleted rows");
}
