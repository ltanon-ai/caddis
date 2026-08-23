//! ledger.rs — append-only JSONL, monotoninis seq (CARD-0001 step 5; v0 failinis).
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

pub struct Ledger {
    file: PathBuf,
    seq: u64,
}

/// CARD-WARDEN-1: escape a string for a JSON string literal.
///
/// The v0 line escaped `\` and `"` only, which held for as long as every body
/// was a short ASCII test string. A warden body carries real tool input — a
/// multi-line bash command, file content — and a RAW newline inside a JSONL
/// record ENDS the record: one append reads back as two lines, the second
/// unparseable, and `open` then recovers `seq` from a fragment.
///
/// Escapes the two structural characters, the five short forms JSON defines,
/// and every remaining C0 control as `\u00XX`. Nothing above U+001F needs
/// escaping in JSON, so text and emoji pass through as themselves (the file is
/// UTF-8 by contract — see rules/common/encoding.md).
fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

impl Ledger {
    pub fn open(file: &Path) -> std::io::Result<Self> {
        let mut seq = 0u64;
        if file.exists() {
            let txt = fs::read_to_string(file)?;
            if let Some(last) = txt.lines().filter(|l| !l.trim().is_empty()).next_back() {
                // minimalus seq paėrimas be serde: "seq":N,
                if let Some(i) = last.find("\"seq\":") {
                    let rest = &last[i + 6..];
                    let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(n) = num.parse::<u64>() {
                        seq = n;
                    }
                }
            }
        }
        Ok(Self {
            file: file.into(),
            seq,
        })
    }
    pub fn append(&mut self, env: &crate::envelope::Envelope) -> std::io::Result<u64> {
        self.seq += 1;
        if let Some(dir) = self.file.parent() {
            fs::create_dir_all(dir)?;
        }
        let mut f = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.file)?;
        writeln!(f, "{{\"seq\":{},\"v\":{},\"id\":\"{}\",\"idem_key\":\"{}\",\"type\":\"{}\",\"from\":\"{}\",\"to\":\"{}\",\"body\":\"{}\",\"ts\":\"{}\"}}",
            self.seq, env.v, esc(&env.id), esc(&env.idem_key), esc(&env.r#type),
            esc(&env.from), esc(&env.to), esc(&env.body), esc(&env.ts))?;
        Ok(self.seq)
    }
    pub fn seq(&self) -> u64 {
        self.seq
    }
}
