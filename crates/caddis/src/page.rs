//! page.rs — CARD-0155. The pager brain: cold store + staged tick.
//! Zero-dep line protocol (estate key=value style). L0 rung only:
//! monotone eviction, one mark + stopping rule + per-cycle cap (D-031).
//! Weighted sums are banned; task-protection is a later ladder rung.

use std::env;
use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use crate::hmac;
use crate::receipt;

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

pub fn run(args: &[String]) -> Result<(), Error> {
    let sub = args
        .first()
        .ok_or_else(|| Error::Usage("page requires a subcommand".into()))?;
    match sub.as_str() {
        "capture" => capture(&args[1..]),
        "tick" => tick(&args[1..]),
        "ref" => ref_cmd(&args[1..]),
        "report" => crate::page_report::report(&args[1..]),
        "mode" => crate::page_mode::run(&args[1..]),
        "mark" => crate::page_mark::run(&args[1..]),
        _ => Err(Error::Usage(format!("unknown page subcommand {sub}"))),
    }
}

pub(crate) fn session_dir(session: &str) -> Result<PathBuf, Error> {
    if session.is_empty() || session.contains('/') || session.contains('\\') {
        return Err(Error::Usage("page --session must be a plain name".into()));
    }
    let home = env::var_os("HOME")
        .or_else(|| env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .ok_or_else(|| Error::Fail("HOME is unset".into()))?;
    Ok(home.join(".caddis").join("pager").join(session))
}

pub fn flag<'a>(args: &'a [String], name: &str) -> Result<&'a str, Error> {
    let prefix = format!("{name}=");
    for (i, a) in args.iter().enumerate() {
        if let Some(v) = a.strip_prefix(&prefix) {
            return Ok(v);
        }
        if a == name {
            return args
                .get(i + 1)
                .map(String::as_str)
                .ok_or_else(|| Error::Usage(format!("missing {name} value")));
        }
    }
    Err(Error::Usage(format!("page requires {name}")))
}

fn opt_num(args: &[String], name: &str, default: u64) -> Result<u64, Error> {
    let prefix = format!("{name}=");
    for (i, a) in args.iter().enumerate() {
        if let Some(v) = a.strip_prefix(&prefix) {
            return v
                .parse()
                .map_err(|_| Error::Usage(format!("{name} must be a number")));
        }
        if a == name {
            let v = args
                .get(i + 1)
                .ok_or_else(|| Error::Usage(format!("missing {name} value")))?;
            return v
                .parse()
                .map_err(|_| Error::Usage(format!("{name} must be a number")));
        }
    }
    Ok(default)
}

fn read_stdin() -> Result<String, Error> {
    let mut s = String::new();
    io::stdin()
        .read_to_string(&mut s)
        .map_err(|e| Error::Fail(format!("stdin: {e}")))?;
    Ok(s)
}

fn field<'a>(block: &'a str, name: &str) -> Option<&'a str> {
    let prefix = format!("{name}=");
    block
        .lines()
        .find(|l| l.starts_with(&prefix))
        .map(|l| &l[prefix.len()..])
}

/// Blocks in cold.store are `---` separated; find the block with this seq.
fn find_block(text: &str, seq: &str) -> Option<String> {
    text.split("\n---\n")
        .find(|b| b.lines().any(|l| l == format!("seq={seq}")))
        .map(str::to_string)
}

struct CaptureFields {
    seq: String,
    role: String,
    chars: String,
    turn: String,
    raw: Vec<u8>,
}

fn parse_capture(body: &str) -> Result<CaptureFields, Error> {
    let text_hex = field(body, "text").ok_or_else(|| Error::Fail("capture: no text".into()))?;
    Ok(CaptureFields {
        seq: field(body, "seq")
            .ok_or_else(|| Error::Fail("capture: no seq".into()))?
            .into(),
        role: field(body, "role").unwrap_or("toolResult").into(),
        chars: field(body, "chars").unwrap_or("0").into(),
        turn: field(body, "turn").unwrap_or("0").into(),
        raw: receipt::decode_hex(text_hex)
            .ok_or_else(|| Error::Fail("capture: text is not hex".into()))?,
    })
}

fn append_record(dir: &Path, f: &CaptureFields) -> Result<(), Error> {
    fs::create_dir_all(dir).map_err(|e| Error::Fail(format!("mkdir {}: {e}", dir.display())))?;
    let path = dir.join("cold.store");
    let existing = fs::read_to_string(&path).unwrap_or_default();
    if find_block(&existing, &f.seq).is_some() {
        println!("capture: seq={} already cold (idempotent)", f.seq);
        return Ok(());
    }
    let sha = receipt::hex_string(&hmac::sha256(&f.raw));
    let record = format!(
        "seq={}\nrole={}\nchars={}\nturn={}\nsha={}\ntext={}\n---\n",
        f.seq,
        f.role,
        f.chars,
        f.turn,
        sha,
        receipt::hex_string(&f.raw)
    );
    let mut file = fs::OpenOptions::new()
        .append(true)
        .create(true)
        .open(&path)
        .map_err(|e| Error::Fail(format!("open {}: {e}", path.display())))?;
    file.write_all(record.as_bytes())
        .map_err(|e| Error::Fail(format!("append: {e}")))?;
    println!("capture: seq={} cold ({} chars)", f.seq, f.chars);
    Ok(())
}

fn capture(args: &[String]) -> Result<(), Error> {
    let dir = session_dir(flag(args, "--session")?)?;
    let fields = parse_capture(&read_stdin()?)?;
    append_record(&dir, &fields)
}

struct Span {
    seq: u64,
    turn: u64,
    chars: u64,
    pinned: bool,
}

fn parse_span(line: &str) -> Result<Span, Error> {
    let p: Vec<&str> = line.split(',').collect();
    if p.len() != 4 {
        return Err(Error::Fail(format!("tick: bad span line: {line}")));
    }
    let num = |s: &str| {
        s.parse::<u64>()
            .map_err(|_| Error::Fail(format!("tick: bad number {s}")))
    };
    Ok(Span {
        seq: num(p[0])?,
        turn: num(p[1])?,
        chars: num(p[2])?,
        pinned: p[3] == "1",
    })
}

fn parse_spans(stdin: &str) -> Result<Vec<Span>, Error> {
    stdin
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(parse_span)
        .collect()
}

/// D-031 admission + ranking: pinned/sticky and the working set are never
/// eligible; below evict_min never; oldest band first, size as tiebreak.
/// CARD-0211: era_turn dissolves stickiness before the era boundary — a
/// pinned span whose turn precedes the boundary becomes evictable. 0 = no
/// dissolution (backward compatible: pinned stays pinned).
fn eligible_sorted(
    spans: &[Span],
    keep_recent: u64,
    evict_min: u64,
    max_turn: u64,
    era_turn: u64,
) -> Vec<&Span> {
    let dissolved = |s: &&Span| !s.pinned || s.turn < era_turn;
    let mut eligible: Vec<&Span> = spans
        .iter()
        .filter(|s| dissolved(s) && s.chars >= evict_min && s.turn + keep_recent <= max_turn)
        .collect();
    eligible.sort_by(|a, b| a.turn.cmp(&b.turn).then(b.chars.cmp(&a.chars)));
    eligible
}

fn emit_evictions(eligible: Vec<&Span>, mut tokens: u64, mark: u64, cap: u64) -> bool {
    for (evicted, s) in eligible.into_iter().enumerate() {
        if tokens <= mark || evicted as u64 >= cap {
            break;
        }
        println!("evict={}", s.seq);
        tokens = tokens.saturating_sub(s.chars.saturating_add(3) / 4);
    }
    tokens > mark
}

fn tick(args: &[String]) -> Result<(), Error> {
    let mark: u64 = flag(args, "--mark-tokens")?
        .parse()
        .map_err(|_| Error::Usage("--mark-tokens must be a number".into()))?;
    let keep_recent = opt_num(args, "--keep-recent", 6)?;
    let evict_min = opt_num(args, "--evict-min", 300)?;
    let cap = opt_num(args, "--cap", u64::MAX)?;
    let era_turn = opt_num(args, "--era-turn", 0)?;
    let spans = parse_spans(&read_stdin()?)?;
    let max_turn = spans.iter().map(|s| s.turn).max().unwrap_or(0);
    let total_tokens = spans
        .iter()
        .map(|s| s.chars.saturating_add(3) / 4)
        .sum::<u64>();
    if total_tokens <= mark {
        println!("starved=false");
        return Ok(());
    }
    let eligible = eligible_sorted(&spans, keep_recent, evict_min, max_turn, era_turn);
    let starved = emit_evictions(eligible, total_tokens, mark, cap);
    println!("starved={starved}");
    Ok(())
}

fn cold_text(dir: &Path, seq: &str) -> Result<Vec<u8>, Error> {
    let store = fs::read_to_string(dir.join("cold.store"))
        .map_err(|e| Error::Fail(format!("no cold.store: {e}")))?;
    let block =
        find_block(&store, seq).ok_or_else(|| Error::Fail(format!("seq {seq} not cold")))?;
    let text_hex =
        field(&block, "text").ok_or_else(|| Error::Fail("cold record has no text".into()))?;
    receipt::decode_hex(text_hex).ok_or_else(|| Error::Fail("cold text is not hex".into()))
}

fn ref_cmd(args: &[String]) -> Result<(), Error> {
    let dir = session_dir(flag(args, "--session")?)?;
    let raw = cold_text(&dir, flag(args, "--seq")?)?;
    let stdout = io::stdout();
    let mut out = stdout.lock();
    out.write_all(&raw)
        .and_then(|_| out.flush())
        .map_err(|e| Error::Fail(format!("write: {e}")))?;
    Ok(())
}
