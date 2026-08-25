//! rows.rs — the ONE ledger-row parser, split from replay.rs under the
//! 280-line law when report joined replay as a second reader. One file
//! format, one parser: a second copy of this would rot apart from the
//! first exactly where the ledger's credibility lives.

pub(crate) struct Row {
    pub(crate) seq: u64,
    pub(crate) from: String,
    pub(crate) ts: u64,
    pub(crate) tool: String,
    pub(crate) body: String,
}

/// Scan a JSONL row for the three fields replay needs — no serde, this
/// crate carries zero dependencies by stated property. Returns None for
/// lines that are not ledger rows; the unescape is the minimal JSON set
/// the ledger writer produces (\", \\, \n, \t).
pub(crate) fn parse_row(line: &str) -> Option<Row> {
    let seq = extract(line, "\"seq\":")?.parse::<u64>().ok()?;
    let typ = extract(line, "\"type\":\"")?;
    let body = extract(line, "\"body\":\"")?;
    let from = unescape(&extract(line, "\"from\":\"").unwrap_or_default());
    let ts = extract(line, "\"ts\":")
        .and_then(|t| t.parse::<u64>().ok())
        .unwrap_or(0);
    Some(Row {
        seq,
        from,
        ts,
        tool: unescape(&typ),
        body: unescape(&body),
    })
}

/// The raw text after `needle` up to the closing quote or digit end.
pub(crate) fn extract(line: &str, needle: &str) -> Option<String> {
    let start = line.find(needle)? + needle.len();
    let rest = &line[start..];
    if needle.ends_with('"') {
        // string field: read to the unescaped closing quote
        let mut out = String::new();
        let mut chars = rest.chars();
        while let Some(c) = chars.next() {
            if c == '\\' {
                out.push(c);
                out.push(chars.next()?);
            } else if c == '"' {
                return Some(out);
            } else {
                out.push(c);
            }
        }
        None
    } else {
        // numeric-or-quoted field: the real ledger quotes ts ("ts":"1787…"),
        // fixtures write it bare — accept both shapes.
        Some(
            rest.split(',')
                .next()?
                .trim_end_matches('}')
                .trim_matches('"')
                .to_string(),
        )
    }
}

pub(crate) fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('t') => out.push('\t'),
            Some('r') => out.push('\r'),
            Some(other) => out.push(other),
            None => out.push('\\'),
        }
    }
    out
}

/// Split `tag|command|path|why` — the command may contain pipes (it is
/// never re-derived from elsewhere), so the tail splits from the RIGHT.
pub(crate) fn split_body(body: &str) -> Option<(String, String)> {
    let (tag, rest) = body.split_once('|')?;
    // rest = "command|path|why": strip why, then path — from the right, so
    // pipes INSIDE the command survive.
    let without_why = rest.rsplit_once('|')?.0;
    let cmd = without_why.rsplit_once('|')?.0;
    Some((tag.to_string(), cmd.to_string()))
}

pub(crate) fn first_line_capped(s: &str) -> String {
    s.lines().next().unwrap_or("").chars().take(60).collect()
}

/// Does this row's caller belong to the caller the reader asked for?
/// (CARD-0109)
///
/// `from` is `<label>` on older rows and `<label>.<session>` on newer ones, so
/// an exact comparison would silently drop every session-scoped row the moment
/// sessions became distinguishable — the operator asks what his session did and
/// is told half of it, with nothing saying anything was withheld.
///
/// THE MATCH IS ON A DOT BOUNDARY, NOT A BARE PREFIX. `starts_with` alone is the
/// obvious wrong fix: it makes `--from peleda` also match a different lane
/// called `peleda-two`, quietly merging two agents' histories into one answer.
pub(crate) fn from_matches(row_from: &str, want: &str) -> bool {
    if row_from == want {
        return true;
    }
    row_from
        .strip_prefix(want)
        .is_some_and(|rest| rest.starts_with('.'))
}

/// The law id a deny reason names (`caddis-warden [id]: …`); text without
/// the bracket form (sensitive-path denials) has no id to group by, and
/// the caller decides what to call that bucket rather than this parser
/// inventing one.
pub(crate) fn law_id_bracketed(why: &str) -> Option<String> {
    match (why.find('['), why.find(']')) {
        (Some(a), Some(b)) if b > a + 1 => Some(why[a + 1..b].to_string()),
        _ => None,
    }
}

#[cfg(test)]
#[path = "rows_tests.rs"]
mod tests;
