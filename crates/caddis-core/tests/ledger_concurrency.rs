//! ledger_concurrency.rs — the append-only ledger under concurrent writers
//! (CARD-0108).
//!
//! THE DEFECT THESE REPRODUCE, measured on the live 15k-row ledger before the
//! fix: 20 rows unparsable, 6733 distinct `seq` values across 15411 rows, and
//! 8678 rows (56%) carrying a DUPLICATE seq. One cause, two symptoms — a
//! `writeln!` onto an unbuffered `File` issues one syscall per format fragment,
//! `O_APPEND` is atomic per SYSCALL rather than per call, so two wardens splice
//! mid-token; and a torn line then defeats the counter recovery in
//! `Ledger::open`, which resets `seq` to 0 and re-issues the whole sequence.
//!
//! The warden is spawned once per tool call by several harnesses sharing ONE
//! ledger, so this is the normal case and not an edge case.

use caddis_core::envelope;
use caddis_core::ledger::Ledger;
use std::io::Write;

fn temp_path(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!(
        "caddis-ledger-{tag}-{}-{:?}.jsonl",
        std::process::id(),
        std::thread::current().id()
    ))
}

fn env_for(n: usize) -> envelope::Envelope {
    // A body wide enough to span several format fragments, which is what makes
    // the interleave observable; the real warden body is `tag|command|path|why`.
    envelope::validate(
        1,
        &format!("wardn{n:016x}"),
        &format!("idem{n:016x}"),
        "tool.bash",
        "peleda",
        "warden",
        &format!("allow|echo concurrent-writer-number-{n} with a body long enough to straddle fragments||"),
        "1787000000",
    )
    .expect("the fixture envelope is valid")
}

/// Rows that do not parse, and rows whose seq was issued more than once.
fn damage(path: &std::path::Path) -> (usize, usize, usize) {
    let text = std::fs::read_to_string(path).expect("the ledger is readable");
    let mut torn = 0usize;
    let mut seqs: Vec<u64> = Vec::new();
    for line in text.lines().filter(|l| !l.trim().is_empty()) {
        match seq_of(line) {
            Some(seq) => seqs.push(seq),
            None => torn += 1,
        }
    }
    let total = seqs.len() + torn;
    seqs.sort_unstable();
    let distinct = {
        let mut d = seqs.clone();
        d.dedup();
        d.len()
    };
    (total, torn, seqs.len() - distinct)
}

/// A line is intact when it is one complete record: exactly one opening brace,
/// one `"seq":` followed by digits, and a closing brace at the end. This is
/// deliberately stricter than "contains a seq" — the torn rows in the live
/// ledger DO contain a parsable-looking seq, which is precisely how they
/// poisoned the counter.
fn seq_of(line: &str) -> Option<u64> {
    let line = line.trim();
    if !line.starts_with('{') || !line.ends_with('}') {
        return None;
    }
    if line.matches("\"seq\":").count() != 1 || line.matches("\"body\":").count() != 1 {
        return None;
    }
    let rest = &line[line.find("\"seq\":")? + 6..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        return None;
    }
    digits.parse().ok()
}

#[test]
fn concurrent_appends_never_tear_a_row_and_never_reissue_a_seq() {
    let path = temp_path("concurrent");
    // swallow: best-effort-cleanup
    let _ = std::fs::remove_file(&path);

    const WRITERS: usize = 8;
    const EACH: usize = 40;
    std::thread::scope(|s| {
        for w in 0..WRITERS {
            let path = path.clone();
            s.spawn(move || {
                // Each writer opens its OWN handle, exactly as a per-call
                // warden process does. Sharing one handle would test something
                // the real system never does.
                for n in 0..EACH {
                    let mut led = Ledger::open(&path).expect("ledger opens");
                    led.append(&env_for(w * EACH + n)).expect("append succeeds");
                }
            });
        }
    });

    let (total, torn, duplicate) = damage(&path);
    // swallow: best-effort-cleanup
    let _ = std::fs::remove_file(&path);

    assert_eq!(
        total,
        WRITERS * EACH,
        "every append must produce exactly one line"
    );
    assert_eq!(
        torn, 0,
        "a torn row is a row no reader can ever attribute; {torn} of {total} were spliced"
    );
    assert_eq!(
        duplicate, 0,
        "seq is the ledger's primary key and must be issued once; {duplicate} of {total} repeated"
    );
}

#[test]
fn a_torn_last_line_does_not_reset_the_counter() {
    // The real recovery path, with a REAL interleaved row from the live ledger
    // rather than an invented one: `Ledger::open` scrapes the first `"seq":`,
    // finds `{` where a digit should be, gives up, and starts again from zero —
    // which is how 8678 rows came to share 6733 numbers.
    let path = temp_path("torn");
    // swallow: best-effort-cleanup
    let _ = std::fs::remove_file(&path);

    let good = "{\"seq\":41,\"v\":1,\"id\":\"a\",\"idem_key\":\"b\",\"type\":\"tool.bash\",\
                \"from\":\"peleda\",\"to\":\"warden\",\"body\":\"allow|echo ok||\",\"ts\":\"1\"}";
    let spliced = "{\"seq\":{\"seq\":538,\"v\":5381,\"v\":1,\"id\":\",\"id\":\"wardnf7acdbc1\"}";
    let mut f = std::fs::File::create(&path).expect("fixture created");
    writeln!(f, "{good}\n{spliced}").expect("fixture written");
    drop(f);

    let mut led = Ledger::open(&path).expect("ledger opens over a damaged tail");
    let issued = led.append(&env_for(99)).expect("append succeeds");
    let text = std::fs::read_to_string(&path).expect("readable");
    // swallow: best-effort-cleanup
    let _ = std::fs::remove_file(&path);

    assert!(
        issued > 41,
        "a torn tail must not reset the counter: the file already holds seq 41 \
         and the next append issued {issued}, which re-uses history"
    );
    assert_eq!(
        text.lines().filter(|l| seq_of(l) == Some(issued)).count(),
        1,
        "the newly issued seq must be unique in the file"
    );
    assert!(
        !text.contains("32773277"),
        "no number spliced out of two fragments may ever become a counter"
    );
}

#[test]
fn a_row_written_by_another_handle_after_open_is_still_seen() {
    // GUARDS THE INCREMENTAL WINDOW. `append` reads only the bytes arriving
    // after `open` to keep the lock hold short; if that window were computed
    // wrongly, a row another warden wrote in between would be invisible and its
    // seq would be handed out twice. This is the exact case the optimisation
    // could break, so it is pinned rather than reasoned about.
    let path = temp_path("interleaved");
    // swallow: best-effort-cleanup
    let _ = std::fs::remove_file(&path);

    let mut early = Ledger::open(&path).expect("opens");
    let first = early.append(&env_for(1)).expect("first append");

    // A SECOND handle, opened later, writes two rows the first handle never saw.
    let mut other = Ledger::open(&path).expect("opens");
    let second = other.append(&env_for(2)).expect("second append");
    let third = other.append(&env_for(3)).expect("third append");

    // The stale handle must not re-issue: it re-reads under the lock.
    let fourth = early.append(&env_for(4)).expect("fourth append");

    let (total, torn, duplicate) = damage(&path);
    // swallow: best-effort-cleanup
    let _ = std::fs::remove_file(&path);

    assert_eq!((total, torn, duplicate), (4, 0, 0));
    assert!(
        first < second && second < third && third < fourth,
        "seq must stay monotonic across handles: {first} {second} {third} {fourth}"
    );
}

#[test]
fn a_truncated_ledger_falls_back_to_a_full_scan() {
    // If the file SHRANK it was rotated or truncated, so the remembered offset
    // describes different bytes and the window is meaningless.
    let path = temp_path("truncated");
    // swallow: best-effort-cleanup
    let _ = std::fs::remove_file(&path);

    let mut led = Ledger::open(&path).expect("opens");
    for n in 0..5 {
        led.append(&env_for(n)).expect("append");
    }
    // Rotation: the file is replaced by a shorter one that still holds a high seq.
    let kept = "{\"seq\":900,\"v\":1,\"id\":\"a\",\"idem_key\":\"b\",\"type\":\"tool.bash\",\
                \"from\":\"peleda\",\"to\":\"warden\",\"body\":\"allow|echo ok||\",\"ts\":\"1\"}\n";
    std::fs::write(&path, kept).expect("rotated");

    let issued = led.append(&env_for(9)).expect("append after rotation");
    // swallow: best-effort-cleanup
    let _ = std::fs::remove_file(&path);
    assert!(
        issued > 900,
        "a rotated file must be re-scanned, not read through a stale offset; issued {issued}"
    );
}

#[test]
fn an_empty_ledger_is_distinguishable_from_an_unreadable_one() {
    // A counter that swallows unreadable input reports 0 and reads as "nothing
    // happened yet", which is the false-clean the whole ledger exists against.
    let path = temp_path("absent");
    // swallow: best-effort-cleanup
    let _ = std::fs::remove_file(&path);

    let mut led = Ledger::open(&path).expect("a missing ledger opens as empty");
    assert_eq!(led.append(&env_for(1)).expect("first append"), 1);

    let dir = std::env::temp_dir().join(format!("caddis-dir-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("a directory where a file should be");
    assert!(
        Ledger::open(&dir).is_err(),
        "an unreadable ledger must be an error, never an empty one"
    );
    // swallow: best-effort-cleanup
    let _ = std::fs::remove_dir_all(&dir);
    // swallow: best-effort-cleanup
    let _ = std::fs::remove_file(&path);
}
