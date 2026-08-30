//! akis_lane.rs — CARD-0271 RED-first. rust-analyzer as an external
//! LANE (fail-open). Today no `caddis akis` exists: exit 2 (usage).
//! Hermetic: a python stub speaking 3 lines of LSP over stdio proves
//! the framing (initialize -> didOpen -> publishDiagnostics with one
//! Error row => akis.jsonl has exactly that row). With the stub
//! absent, exit 0 + "lane down". A parse-error file through the stub
//! yields severity=Error advisory rows and NO gate effect (exit 0).

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};

static SEQ: AtomicU64 = AtomicU64::new(0);

/// Python stub speaking 3 lines of LSP over Content-Length-framed
/// stdio: initialize response, then (after didOpen) one
/// publishDiagnostics notification. `--version` probes the lane up.
const STUB: &str = "\
import sys, json
def read_msg():
    n = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if line in (b'\\r\\n', b'\\n'):
            break
        if line.startswith(b'Content-Length:'):
            n = int(line.split(b':')[1].strip())
    if n is None:
        return None
    return json.loads(sys.stdin.buffer.read(n).decode())
def write_msg(o):
    d = json.dumps(o).encode()
    sys.stdout.buffer.write(('Content-Length: %d\\r\\n\\r\\n' % len(d)).encode())
    sys.stdout.buffer.write(d)
    sys.stdout.buffer.flush()
if '--version' in sys.argv[1:]:
    print('akis-stub 1.0')
    sys.exit(0)
read_msg()
write_msg({'jsonrpc':'2.0','id':1,'result':{'capabilities':{'textDocumentSync':1}}})
text, uri = '', ''
while True:
    m = read_msg()
    if m is None:
        break
    if m.get('method') == 'textDocument/didOpen':
        text = m['params']['textDocument']['text']
        uri = m['params']['textDocument']['uri']
        break
if 'PARSE' in text.upper():
    diag = {'range':{'start':{'line':0,'character':0},'end':{'line':0,'character':1}},'severity':1,'code':'E_PARSE','message':'parse error'}
else:
    diag = {'range':{'start':{'line':2,'character':0},'end':{'line':2,'character':5}},'severity':1,'code':'E0001','message':'stub diagnostic'}
write_msg({'jsonrpc':'2.0','method':'textDocument/publishDiagnostics','params':{'uri':uri,'diagnostics':[diag]}})
";

fn tmp(tag: &str) -> PathBuf {
    let n = SEQ.fetch_add(1, Ordering::Relaxed);
    let p = std::env::temp_dir().join(format!("caddis-akis-{tag}-{n}"));
    let _ = fs::remove_dir_all(&p);
    fs::create_dir_all(&p).unwrap();
    p
}

struct World {
    cwd: PathBuf,
    stub: PathBuf,
}

impl World {
    fn new(tag: &str) -> Self {
        let cwd = tmp(tag);
        let stub = cwd.join("stub.py");
        fs::write(&stub, STUB).unwrap();
        Self { cwd, stub }
    }

    fn akis(&self, akis_bin: &str, args: &[&str]) -> (String, String, i32) {
        let out = Command::new(env!("CARGO_BIN_EXE_caddis"))
            .args(args)
            .current_dir(&self.cwd)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .env("CADDIS_AKIS_BIN", akis_bin)
            .output()
            .expect("spawn caddis");
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
            out.status.code().unwrap_or(-1),
        )
    }

    fn stub_bin(&self) -> String {
        format!("python {}", self.stub.display())
    }
}

/// RED: stub present, one didOpen -> akis.jsonl has exactly one Error
/// row. Today `caddis akis` is exit 2 (no subcommand).
#[test]
fn framing_one_error_row_in_jsonl() {
    let w = World::new("framing");
    let src = w.cwd.join("src.rs");
    fs::write(&src, "fn main() {}\n").unwrap();
    let (stdout, stderr, code) = w.akis(
        &w.stub_bin(),
        &[
            "akis",
            "--card",
            "CARD-0271",
            "--file",
            src.to_str().unwrap(),
        ],
    );
    assert_eq!(
        code, 0,
        "akis must exit 0 (advisory); got {code}\nstdout: {stdout}\nstderr: {stderr}"
    );
    let jsonl = fs::read_to_string(w.cwd.join("akis.jsonl")).expect("akis.jsonl written");
    assert!(
        jsonl.contains("\"severity\":\"Error\""),
        "must have an Error row: {jsonl}"
    );
    assert!(
        jsonl.contains("\"code\":\"E0001\""),
        "must carry the stub code: {jsonl}"
    );
    assert_eq!(jsonl.lines().count(), 1, "exactly one row: {jsonl}");
}

/// RED: lane absent (probe fails) -> exit 0 + "lane down", no jsonl.
#[test]
fn lane_down_exits_zero() {
    let w = World::new("down");
    let (stdout, _stderr, code) =
        w.akis("nonexistent-akis-bin-xyz", &["akis", "--card", "CARD-0271"]);
    assert_eq!(code, 0, "lane down must exit 0; got {code}");
    assert!(
        stdout.contains("lane down"),
        "must report lane down: {stdout}"
    );
    assert!(
        !w.cwd.join("akis.jsonl").exists(),
        "no jsonl when lane down"
    );
}

/// RED: a parse-error file yields Error advisory rows but exit 0 —
/// nits never gate; the spine owns hard gates.
#[test]
fn parse_error_is_advisory_no_gate() {
    let w = World::new("parse");
    let src = w.cwd.join("bad.rs");
    fs::write(&src, "PARSE fn broken(\n").unwrap();
    let (stdout, stderr, code) = w.akis(
        &w.stub_bin(),
        &[
            "akis",
            "--card",
            "CARD-0271",
            "--file",
            src.to_str().unwrap(),
        ],
    );
    assert_eq!(
        code, 0,
        "parse-error must NOT gate (exit 0); got {code}\nstdout: {stdout}\nstderr: {stderr}"
    );
    let jsonl = fs::read_to_string(w.cwd.join("akis.jsonl")).expect("akis.jsonl written");
    assert!(
        jsonl.contains("\"severity\":\"Error\""),
        "parse-error yields Error rows: {jsonl}"
    );
    assert!(jsonl.contains("E_PARSE"), "parse-error code: {jsonl}");
}
