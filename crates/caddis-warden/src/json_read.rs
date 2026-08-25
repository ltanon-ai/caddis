//! json_read.rs — the handful of readers `attest --verify` needs to pull
//! fields back out of a bundle, split from `attest_verify.rs` under the
//! 280-line law (CARD-0114).
//!
//! Hand-rolled under the crate's zero-dependency law, same as the writer.
//!
//! ⛔ THESE WALK THE STRUCTURE. THEY MUST NOT COUNT CHARACTERS, AND TWO EARLIER
//! ATTEMPTS DID. `arr_len` counted quotes/2 and `obj_len` counted colons, which
//! holds only while the data is tame — and this data is PATHS. One Windows key
//! like `"C:/w/src/a.rs"` carries a colon, so the object reader answered 2 for a
//! single entry and an HONEST bundle contradicted itself on `files_distinct`.
//! Every fixture in this crate used relative paths, so nothing caught it; the
//! mandatory pre-push review did.
//!
//! A verifier that raises a FALSE contradiction is worse than useless: it
//! teaches its reader to ignore it, and then the true contradiction goes unread
//! as well.

/// Where a container's body starts, just past `"key":<open>`.
fn body_start(json: &str, key: &str, open: char) -> Option<usize> {
    Some(json.find(&format!("\"{key}\":{open}"))? + key.len() + 4)
}

/// One pass of the scanner: string state, escape state, nesting depth.
#[derive(Default)]
struct Scan {
    depth: usize,
    in_string: bool,
    escaped: bool,
    commas: usize,
    saw_content: bool,
}

impl Scan {
    /// Feed one character. `Some(n)` when the container just closed.
    fn step(&mut self, c: char) -> Option<usize> {
        if self.escaped {
            self.escaped = false;
            return None;
        }
        if self.in_string {
            match c {
                '\\' => self.escaped = true,
                '"' => self.in_string = false,
                _ => {}
            }
            return None;
        }
        self.structural(c)
    }

    /// A character OUTSIDE a string, where punctuation means structure.
    fn structural(&mut self, c: char) -> Option<usize> {
        match c {
            '"' => {
                self.in_string = true;
                self.saw_content = true;
            }
            '[' | '{' => self.depth += 1,
            ']' | '}' => {
                self.depth -= 1;
                if self.depth == 0 {
                    return Some(if self.saw_content { self.commas + 1 } else { 0 });
                }
            }
            ',' if self.depth == 1 => self.commas += 1,
            c if !c.is_whitespace() => self.saw_content = true,
            _ => {}
        }
        None
    }
}

/// How many top-level entries the container at `key` holds.
///
/// Respects string state and escapes, so a `:`, `,` or `"` INSIDE a key or a
/// value counts as data rather than as structure. `None` means the container
/// never closed — a malformed bundle, which must read as unreadable rather than
/// as a number, because `(absent)` never compares equal to a real count and the
/// claim therefore reports CONTRADICTED.
fn count_entries(json: &str, key: &str, open: char) -> Option<usize> {
    let at = body_start(json, key, open)?;
    let mut scan = Scan {
        depth: 1,
        ..Default::default()
    };
    json[at..].chars().find_map(|c| scan.step(c))
}

pub fn arr_len(json: &str, key: &str) -> Option<usize> {
    count_entries(json, key, '[')
}

pub fn obj_len(json: &str, key: &str) -> Option<usize> {
    count_entries(json, key, '{')
}

/// A number field, by name.
pub fn num(json: &str, key: &str) -> Option<u64> {
    let at = json.find(&format!("\"{key}\":"))? + key.len() + 3;
    json[at..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

/// A string field, by name. Stops at the first UNESCAPED quote, so a value
/// containing `\"` survives whole.
pub fn text_field(json: &str, key: &str) -> Option<String> {
    let at = json.find(&format!("\"{key}\":\""))? + key.len() + 4;
    let mut out = String::new();
    let mut escaped = false;
    for c in json[at..].chars() {
        if escaped {
            out.push(c);
            escaped = false;
            continue;
        }
        match c {
            '\\' => escaped = true,
            '"' => return Some(out),
            _ => out.push(c),
        }
    }
    None
}

#[cfg(test)]
#[path = "json_read_tests.rs"]
mod tests;
