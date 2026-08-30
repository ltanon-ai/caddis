//! size.rs — CARD-ALL-TOOLCALL-1. A write/edit that would make a file
//! exceed 280 lines or a function exceed CCN 10 is DENY at the tool
//! call. Commit is too late; the file would already exist.

use crate::{ToolCall, Verdict};

const MAX_LINES: usize = 280;
const MAX_CCN: u32 = 10;

/// `None` = this law does not apply. `Some(Deny)` = do not run the tool.
pub fn check(call: &ToolCall) -> Option<Verdict> {
    if !is_file_write(&call.tool) {
        return None;
    }
    let n = resulting_lines(call);
    if n > MAX_LINES {
        return Some(Verdict::Deny {
            reason: format!(
                "caddis-warden: write/edit would make the file {n} lines; the cap is {MAX_LINES}. Split the file first."
            ),
        });
    }
    let ccn = max_function_ccn(call.path.as_str(), &call.content)?;
    if ccn > MAX_CCN {
        return Some(Verdict::Deny {
            reason: format!(
                "caddis-warden: a function CCN {ccn} exceeds {MAX_CCN}. Split the function first."
            ),
        });
    }
    None
}

fn is_file_write(tool: &str) -> bool {
    let t = tool.to_ascii_lowercase();
    t == "write" || t == "edit"
}

fn line_count(s: &str) -> usize {
    if s.is_empty() {
        0
    } else {
        s.lines().count()
    }
}

fn resulting_lines(call: &ToolCall) -> usize {
    let written = line_count(&call.content);
    if call.tool.eq_ignore_ascii_case("write") {
        return written;
    }
    let existing = std::fs::read_to_string(&call.path)
        .map(|s| line_count(&s))
        .unwrap_or(0);
    written.max(existing)
}

fn max_function_ccn(path: &str, src: &str) -> Option<u32> {
    let lang = lang(path)?;
    let bodies = functions(lang, src);
    if bodies.is_empty() {
        return None;
    }
    bodies.iter().map(|b| ccn(b)).max()
}

#[derive(Clone, Copy)]
enum Lang {
    Rust,
    Python,
    Js,
}

fn lang(path: &str) -> Option<Lang> {
    let p = path.to_ascii_lowercase();
    if p.ends_with(".rs") {
        Some(Lang::Rust)
    } else if p.ends_with(".py") {
        Some(Lang::Python)
    } else if p.ends_with(".ts") || p.ends_with(".js") || p.ends_with(".tsx") || p.ends_with(".jsx")
    {
        Some(Lang::Js)
    } else {
        None
    }
}

fn functions(lang: Lang, src: &str) -> Vec<String> {
    match lang {
        Lang::Python => python_fns(src),
        Lang::Rust | Lang::Js => brace_fns(src, lang),
    }
}

fn brace_fns(src: &str, lang: Lang) -> Vec<String> {
    let mut out = Vec::new();
    let b = src.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let hit = match lang {
            Lang::Rust => is_word_at(b, i, b"fn"),
            Lang::Js => is_word_at(b, i, b"function"),
            Lang::Python => false,
        };
        if hit {
            if let Some(rel) = src[i..].find('{') {
                let start = i + rel;
                if let Some(end) = matching_brace(src, start) {
                    out.push(src[i..=end].to_string());
                    i = end + 1;
                    continue;
                }
            }
        }
        i += 1;
    }
    out
}

fn python_fns(src: &str) -> Vec<String> {
    let lines: Vec<&str> = src.lines().collect();
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        let raw = lines[i];
        let trimmed = raw.trim_start();
        if trimmed.starts_with("def ") || trimmed.starts_with("async def ") {
            let indent = raw.len() - trimmed.len();
            let mut j = i + 1;
            while j < lines.len() {
                let n = lines[j];
                if n.trim().is_empty() {
                    j += 1;
                    continue;
                }
                let ni = n.len() - n.trim_start().len();
                if ni <= indent {
                    break;
                }
                j += 1;
            }
            out.push(lines[i..j].join("\n"));
            i = j;
            continue;
        }
        i += 1;
    }
    out
}

fn matching_brace(src: &str, open: usize) -> Option<usize> {
    let b = src.as_bytes();
    if open >= b.len() || b[open] != b'{' {
        return None;
    }
    let mut depth = 0i32;
    for (i, &c) in b.iter().enumerate().skip(open) {
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(i);
                }
            }
            _ => {}
        }
    }
    None
}

fn is_word_at(b: &[u8], i: usize, word: &[u8]) -> bool {
    if i + word.len() > b.len() {
        return false;
    }
    if i > 0 && is_ident(b[i - 1]) {
        return false;
    }
    if &b[i..i + word.len()] != word {
        return false;
    }
    let after = i + word.len();
    after == b.len() || !is_ident(b[after])
}

fn is_ident(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_'
}

fn ccn(body: &str) -> u32 {
    let mut n = 1u32;
    let b = body.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i].is_ascii_alphabetic() {
            let start = i;
            while i < b.len() && is_ident(b[i]) {
                i += 1;
            }
            match &body[start..i] {
                "if" | "elif" | "for" | "while" | "match" | "case" | "catch" | "except" | "and"
                | "or" => n += 1,
                _ => {}
            }
        } else if (b[i] == b'&' && b.get(i + 1) == Some(&b'&'))
            || (b[i] == b'|' && b.get(i + 1) == Some(&b'|'))
        {
            n += 1;
            i += 2;
        } else if b[i] == b'?' {
            n += 1;
            i += 1;
        } else {
            i += 1;
        }
    }
    n
}
