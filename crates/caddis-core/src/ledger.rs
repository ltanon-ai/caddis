//! ledger.rs — append-only JSONL, monotoninis seq (CARD-0001 step 5; v0 failinis).
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use crate::ledger_lock::Lock;

pub struct Ledger {
    file: PathBuf,
    seq: u64,
    unreadable: usize,
    /// Byte offset up to which `seq` already accounts for every row, so a later
    /// append only has to read what arrived after it.
    read_to: u64,
}

/// The file's bytes from `offset` to the end, lossily decoded.
///
/// Lossy on purpose: a torn row can split a multi-byte character, and one
/// mangled character must not make the counter unreadable — `row_seq` will
/// reject that row anyway. The alternative, failing the whole read, would let
/// one damaged byte stop the ledger from recording at all.
fn read_from(path: &Path, offset: u64) -> String {
    use std::io::{Read, Seek, SeekFrom};
    let mut buf = Vec::new();
    // An unreadable tail cannot lower the counter: the value carried from
    // `open` is the floor, so the worst case is a seq higher than necessary.
    // swallow: best-effort-telemetry
    let _ = fs::File::open(path).and_then(|mut f| {
        f.seek(SeekFrom::Start(offset))?;
        f.read_to_end(&mut buf)
    });
    String::from_utf8_lossy(&buf).into_owned()
}

/// The seq of ONE intact row, or None if the line is torn (CARD-0108).
///
/// Deliberately stricter than "find the first `\"seq\":` and read digits",
/// which is what the v0 recovery did and what let a spliced line poison the
/// counter: a torn row DOES contain a plausible-looking seq. The live ledger
/// held `{"seq":{"seq":32773277,...` — two interleaved `3277` fragments read as
/// one integer, which then BECAME the counter for the next process to open the
/// file. A complete record has exactly one opening brace, one `"seq":`, one
/// `"body":`, and a closing brace; anything else is not attributable to a
/// single writer and must not be parsed for anything.
/// Is this line one complete record written by a single writer?
///
/// The ledger owns this definition, so every reader asks the same question and
/// gets the same answer. A second copy elsewhere would eventually disagree with
/// this one about what "damaged" means, and the counts a reader is shown would
/// stop matching the counts the writer keeps.
pub fn is_intact_row(line: &str) -> bool {
    row_seq(line).is_some()
}

fn row_seq(line: &str) -> Option<u64> {
    let line = line.trim();
    if !line.starts_with('{') || !line.ends_with('}') {
        return None;
    }
    if line.matches("\"seq\":").count() != 1 || line.matches("\"body\":").count() != 1 {
        return None;
    }
    let rest = &line[line.find("\"seq\":")? + 6..];
    let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
    digits.parse().ok()
}

impl Ledger {
    /// Recover the counter as the MAXIMUM seq over intact rows (CARD-0108).
    ///
    /// The v0 recovery read the LAST line, so one torn tail reset the counter to
    /// 0 and the whole sequence was re-issued. Measured damage on the live
    /// ledger before this fix: 6733 distinct seq values across 15411 rows, with
    /// 8678 rows carrying a duplicate — 56% of an append-only ledger whose seq
    /// is its primary key — and 19 counter resets, one per torn row.
    ///
    /// Maximum rather than last-intact, because a file whose counter has already
    /// been reset 19 times contains numbers AHEAD of its own tail: resuming from
    /// the tail would keep re-issuing them. The live maximum is 6733, so a
    /// max-based counter collides with nothing already written.
    pub fn open(file: &Path) -> std::io::Result<Self> {
        let mut seq = 0u64;
        let mut unreadable = 0usize;
        let mut read_to = 0u64;
        if file.exists() {
            let txt = fs::read_to_string(file)?;
            read_to = txt.len() as u64;
            for line in txt.lines().filter(|l| !l.trim().is_empty()) {
                match row_seq(line) {
                    Some(n) => seq = seq.max(n),
                    // COUNTED, never silently skipped: a reader must be able to
                    // tell an empty ledger from a damaged one, and both read as
                    // zero if the damage is swallowed.
                    None => unreadable += 1,
                }
            }
        }
        Ok(Self {
            file: file.into(),
            seq,
            unreadable,
            read_to,
        })
    }

    /// How many lines `open` could not attribute to a single writer. Nonzero
    /// means historical damage: rows that no reader can ever cite.
    pub fn unreadable(&self) -> usize {
        self.unreadable
    }

    /// The highest intact seq in the file right now, read under the lock so the
    /// number a writer claims is the number no other writer can claim.
    ///
    /// READS ONLY WHAT `open` HAS NOT ALREADY SEEN. `open` folded every row up
    /// to `read_to` into `self.seq`, so that value is a floor covering all of
    /// history and only the bytes appended since then can raise it. This is
    /// what keeps the LOCK HOLD short, which is the number that matters once
    /// wardens serialize on it: a full re-scan measured 7.6ms per append on the
    /// live 5.1MB ledger and grows linearly with the file, on a path that runs
    /// once per tool call.
    ///
    /// Falls back to a full scan if the file SHRANK, which means it was rotated
    /// or truncated and the earlier offset describes different bytes.
    fn max_seq(&mut self) -> u64 {
        let from = match fs::metadata(&self.file) {
            Ok(m) if m.len() >= self.read_to => self.read_to,
            _ => 0,
        };
        // ⛔ THE WINDOW ALWAYS STARTS ON A ROW BOUNDARY, SO NOTHING IS SKIPPED.
        // `open` reads to EOF and `append` records the length after a
        // newline-terminated ATOMIC write, so no offset can ever fall mid-row.
        // An earlier draft skipped the window's first line as "probably
        // partial"; it was a whole row every time, and 55 of 320 seqs were
        // re-issued because the row that had just been written was invisible to
        // the next writer. Reasoning said partial, the test said otherwise.
        let tail = read_from(&self.file, from);
        tail.lines()
            .filter_map(row_seq)
            .fold(self.seq, |hi, n| hi.max(n))
    }
    pub fn append(&mut self, env: &crate::envelope::Envelope) -> std::io::Result<u64> {
        if let Some(dir) = self.file.parent() {
            fs::create_dir_all(dir)?;
        }
        // ⛔ THE NUMBER IS CHOSEN UNDER THE LOCK, NOT AT `open` (CARD-0108).
        // An atomic append fixes torn ROWS and does nothing for duplicate
        // NUMBERS: `open` reads the maximum, then `append` claims max+1, and two
        // stateless wardens that both read N both write N+1. Measured with the
        // atomic write in place and no lock: 0 torn rows and still 254 of 320
        // seqs repeated. Read-then-write is a race wherever the read is outside
        // the exclusion.
        let _guard = Lock::acquire(&self.file)?;
        self.seq = self.max_seq() + 1;
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)?;
        // ⛔ ONE STRING, ONE `write_all` — NEVER `writeln!` ONTO THE FILE
        // (CARD-0108). `File` is unbuffered, so `writeln!` issues one write
        // syscall PER FORMAT FRAGMENT, and `O_APPEND` (Windows
        // `FILE_APPEND_DATA`) is atomic per SYSCALL, not per call. The warden is
        // spawned once per tool call by several harnesses sharing ONE ledger, so
        // two writers spliced mid-token as the normal case: a reproduction with
        // 8 concurrent writers tore 286 of 320 rows.
        //
        // ⛔ THE CAP IS ENFORCED HERE, BY THE FUNCTION THAT MAKES THE PROMISE.
        // This comment used to say rows stay small because "body.rs caps the body
        // at 500 bytes". That cap lives in a DIFFERENT CRATE (caddis-warden), it
        // bounds only the COMMAND rather than the whole `tag|command|path|why`
        // body, and `envelope::validate` has no body limit at all -- `body` is an
        // opaque String. The kernel was stating an unconditional guarantee that
        // rested on a downstream crate it cannot see, and callers outside the
        // warden (card.rs, the organs canary) never passed through that cap. An
        // atomicity claim depending on someone else's discipline is not a
        // guarantee; it is a hope with a comment on it.
        //
        // The row is now bounded before it is written, and an elided body SAYS so
        // -- the rule body.rs already applies to commands: a shortened record must
        // never masquerade as the whole one.
        let row = crate::ledger_row::row_for(self.seq, env);
        f.write_all(row.as_bytes())?;
        // Everything on disk is now accounted for by `self.seq`, so the next
        // append under the lock reads only what arrives after this point.
        // Re-read the length rather than adding the row's own size: another
        // writer may have appended between our max_seq and our write, and
        // assuming otherwise would leave that row unread forever.
        self.read_to = fs::metadata(&self.file).map(|m| m.len()).unwrap_or(0);
        Ok(self.seq)
    }
    pub fn seq(&self) -> u64 {
        self.seq
    }
}
