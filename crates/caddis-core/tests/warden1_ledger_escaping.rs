//! CARD-WARDEN-1 — the ledger must survive an ARBITRARY body, because the
//! consciousness is about to be fed real tool calls.
//!
//! WHY THIS CARD EXISTS: `Ledger::append` hand-builds its JSON line and escapes
//! exactly two characters (`\` and `"`). Every envelope body it has carried so
//! far was a short ASCII test string, so the gap never showed. The moment the
//! warden envelopes an omp `bash` call, the body carries NEWLINES (a heredoc, a
//! multi-line command, file content) — and a raw newline inside a JSONL record
//! ends the record. One appended envelope then reads back as two lines, the
//! second of them unparsable.
//!
//! That is not a cosmetic defect. An append-only ledger whose lines cannot be
//! parsed is not an audit trail; and `Ledger::open` recovers `seq` by reading
//! the LAST line, so a split record also corrupts the sequence on the next boot.
//! The ledger is the one artifact the whole trust argument rests on.

use caddis_core::envelope::validate;
use caddis_core::ledger::Ledger;

fn tmp(name: &str) -> std::path::PathBuf {
    let mut p = std::env::temp_dir();
    p.push(format!(
        "caddis-warden1-{}-{}.jsonl",
        name,
        std::process::id()
    ));
    let _ = std::fs::remove_file(&p);
    p
}

/// A body containing a newline must still produce EXACTLY ONE ledger line.
#[test]
fn a_body_with_a_newline_stays_one_ledger_line() {
    let path = tmp("newline");
    let mut led = Ledger::open(&path).expect("open");

    // Exactly the shape omp hands us: a multi-line bash command.
    let body = "echo one\necho two";
    let env = validate(
        1,
        "id-warden-0001",
        "idem-newline-1",
        "tool.bash",
        "omp",
        "warden",
        body,
        "2026-08-23T02:00:00Z",
    )
    .expect("valid envelope");

    led.append(&env).expect("append");

    let txt = std::fs::read_to_string(&path).expect("read");
    let lines: Vec<&str> = txt.lines().filter(|l| !l.trim().is_empty()).collect();
    assert_eq!(
        lines.len(),
        1,
        "one appended envelope must be ONE line; a raw newline split the record:\n{txt}"
    );
}

/// And that one line must be parseable JSON with the body round-tripping intact.
/// Parsed by hand (caddis-core carries no serde dependency by design).
#[test]
fn the_line_round_trips_the_body_verbatim() {
    let path = tmp("roundtrip");
    let mut led = Ledger::open(&path).expect("open");

    // Newline, tab, quote, backslash and a control char — all legal in a body,
    // all illegal RAW inside a JSON string.
    let body = "a\nb\tc\"d\\e\u{1}f";
    let env = validate(
        1,
        "id-warden-0002",
        "idem-roundtrip-1",
        "tool.bash",
        "omp",
        "warden",
        body,
        "2026-08-23T02:00:00Z",
    )
    .expect("valid envelope");
    led.append(&env).expect("append");

    let txt = std::fs::read_to_string(&path).expect("read");
    let line = txt.lines().find(|l| !l.trim().is_empty()).expect("a line");

    // Locate "body":" ... " and decode the JSON string escapes.
    let key = "\"body\":\"";
    let start = line.find(key).expect("body key present") + key.len();
    let mut out = String::new();
    let mut it = line[start..].chars();
    loop {
        match it.next().expect("unterminated body string") {
            '"' => break,
            '\\' => match it.next().expect("dangling escape") {
                'n' => out.push('\n'),
                't' => out.push('\t'),
                'r' => out.push('\r'),
                '"' => out.push('"'),
                '\\' => out.push('\\'),
                'u' => {
                    let hex: String = (0..4).map(|_| it.next().expect("short \\u")).collect();
                    let cp = u32::from_str_radix(&hex, 16).expect("hex");
                    out.push(char::from_u32(cp).expect("scalar"));
                }
                other => panic!("unknown escape \\{other} in ledger line: {line}"),
            },
            c => out.push(c),
        }
    }
    assert_eq!(
        out, body,
        "the body must survive the ledger verbatim: {line}"
    );
}

/// The regression that a split record causes downstream: `seq` is recovered from
/// the LAST line, so a broken record silently resets the sequence on reopen.
/// A positive control guards this: seq must be 2 BEFORE we assert it survives.
#[test]
fn seq_survives_a_reopen_after_a_multiline_body() {
    let path = tmp("seq");
    {
        let mut led = Ledger::open(&path).expect("open");
        for (n, body) in [("idem-seq-1", "plain"), ("idem-seq-2", "has\nnewline")] {
            let env = validate(
                1,
                "id-warden-0003",
                n,
                "tool.bash",
                "omp",
                "warden",
                body,
                "2026-08-23T02:00:00Z",
            )
            .expect("valid envelope");
            led.append(&env).expect("append");
        }
        assert_eq!(
            led.seq(),
            2,
            "positive control: two appends must reach seq 2"
        );
    }
    let reopened = Ledger::open(&path).expect("reopen");
    assert_eq!(
        reopened.seq(),
        2,
        "seq is read from the last line; a split record corrupts it"
    );
}
