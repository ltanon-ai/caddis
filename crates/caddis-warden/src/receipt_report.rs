//! receipt_report.rs — how a receipt READS, split from how it is computed
//! (CARD-0112, the seam CARD-0107 established for `replay`).
//!
//! Pure functions returning a String, so the rendering — which is the part that
//! can mislead — is directly assertable without capturing a global stream.

use crate::receipt::Receipt;
use crate::wire::json_escape;

/// How many distinct files are itemized before the list is summarised.
const FILES_SHOWN: usize = 20;

pub fn render_text(r: &Receipt) -> String {
    let mut s = format!("receipt: {}", r.ledger);
    s.push_str(&format!(
        "\nscope: from={} since={}",
        r.from.clone().unwrap_or_else(|| "(everyone)".into()),
        r.since_hours
            .map(|h| format!("{h}h"))
            .unwrap_or_else(|| "(all history)".into())
    ));
    if r.rows == 0 {
        // A legitimate answer, not an error: nothing happened in this window.
        s.push_str("\nNOTHING IN THIS WINDOW — no rows matched.");
        s.push_str(&coverage_line(r));
        return s;
    }
    s.push_str(&format!(
        "\nwindow: ts {} .. {}\nrows: {}  allow: {}  steer: {}  deny: {}",
        r.first_ts.unwrap_or(0),
        r.last_ts,
        r.rows,
        r.allow,
        r.steer,
        r.deny
    ));
    s.push_str(&section("tools", &counts(&r.by_tool)));
    s.push_str(&files_section(r));
    s.push_str(&denials_section(r));
    s.push_str(&section("laws fired", &counts(&r.law_fires)));
    s.push_str(&cards_section(r));
    s.push_str(&coverage_line(r));
    s
}

/// ⛔ ALWAYS PRINTED, INCLUDING WHEN BOTH ARE ZERO. A receipt that hides what it
/// could not read looks exactly like a receipt over a clean window, and a
/// handoff auditor diffing prose against it would call the missing rows a
/// fabrication — or miss a real one.
fn coverage_line(r: &Receipt) -> String {
    // ⚠ THE TWO NUMBERS HAVE DIFFERENT SCOPES AND THE LINE SAYS SO. A torn row
    // carries no readable timestamp or caller, so it cannot be placed in a
    // window at all — the unreadable count is therefore FILE-WIDE, while the
    // withheld count is inside the window like everything else. Printing them
    // side by side without that distinction would invite a reader to subtract
    // one from the other.
    format!(
        "\ncoverage: {} withheld command(s) IN THIS WINDOW (masked or elided — \
         recorded as having happened, contents not kept); {} unreadable line(s) \
         FILE-WIDE (a torn row has no window to belong to)",
        r.withheld, r.unreadable
    )
}

fn counts(m: &std::collections::BTreeMap<String, u64>) -> Vec<String> {
    m.iter().map(|(k, v)| format!("{k}={v}")).collect()
}

fn section(title: &str, items: &[String]) -> String {
    if items.is_empty() {
        return String::new();
    }
    format!("\n{title}: {}", items.join("  "))
}

fn files_section(r: &Receipt) -> String {
    if r.files.is_empty() {
        return String::new();
    }
    let mut s = format!("\nfiles written: {} distinct", r.files.len());
    for (path, n) in r.files.iter().take(FILES_SHOWN) {
        s.push_str(&format!("\n  {path} (x{n})"));
    }
    if r.files.len() > FILES_SHOWN {
        // Say how many were not shown rather than truncating in silence.
        s.push_str(&format!(
            "\n  ... and {} more not shown",
            r.files.len() - FILES_SHOWN
        ));
    }
    s
}

fn denials_section(r: &Receipt) -> String {
    if r.deny_by_law.is_empty() {
        return String::new();
    }
    let mut s = String::from("\ndenials by law:");
    for (law, seqs) in &r.deny_by_law {
        let cited: Vec<String> = seqs.iter().map(u64::to_string).collect();
        s.push_str(&format!(
            "\n  {law} x{} (seq {})",
            seqs.len(),
            cited.join(",")
        ));
    }
    s
}

fn cards_section(r: &Receipt) -> String {
    if r.cards_opened.is_empty() && r.cards_closed.is_empty() {
        return String::new();
    }
    let still_open: Vec<&String> = r
        .cards_opened
        .iter()
        .filter(|id| !r.cards_closed.contains(id))
        .collect();
    let mut s = format!(
        "\ncards: {} opened, {} closed",
        r.cards_opened.len(),
        r.cards_closed.len()
    );
    if !still_open.is_empty() {
        let names: Vec<&str> = still_open.iter().map(|s| s.as_str()).collect();
        s.push_str(&format!("\n  STILL OPEN: {}", names.join(", ")));
    }
    s
}

pub fn render_json(r: &Receipt) -> String {
    format!(
        "{{\"ledger\":\"{}\",\"from\":{},\"since_hours\":{},\"rows\":{},\"verdicts\":{{\"allow\":{},\"steer\":{},\"deny\":{}}},\
         \"first_ts\":{},\"last_ts\":{},\"tools\":{{{}}},\"files\":{{{}}},\
         \"deny_by_law\":{{{}}},\"law_fires\":{{{}}},\
         \"cards_opened\":[{}],\"cards_closed\":[{}],\
         \"unreadable\":{},\"withheld\":{}}}",
        json_escape(&r.ledger),
        // The SCOPE the receipt covers. render_text has printed it since the
        // beginning and render_json dropped it, so a JSON consumer could not tell
        // a whole-ledger receipt from a one-caller one-hour slice — two readers of
        // the same struct disagreeing about which window they describe. `null` is
        // the honest rendering of unset; an empty string would read as a real
        // filter that matched nothing.
        r.from
            .as_ref()
            .map(|f| format!("\"{}\"", json_escape(f)))
            .unwrap_or_else(|| "null".into()),
        r.since_hours
            .map(|h| h.to_string())
            .unwrap_or_else(|| "null".into()),
        r.rows,
        r.allow,
        r.steer,
        r.deny,
        r.first_ts.unwrap_or(0),
        r.last_ts,
        obj(&r.by_tool),
        obj(&r.files),
        seq_obj(&r.deny_by_law),
        obj(&r.law_fires),
        arr(&r.cards_opened),
        arr(&r.cards_closed),
        r.unreadable,
        r.withheld
    )
}

fn obj(m: &std::collections::BTreeMap<String, u64>) -> String {
    m.iter()
        .map(|(k, v)| format!("\"{}\":{}", json_escape(k), v))
        .collect::<Vec<_>>()
        .join(",")
}

fn seq_obj(m: &std::collections::BTreeMap<String, Vec<u64>>) -> String {
    m.iter()
        .map(|(k, v)| {
            let s: Vec<String> = v.iter().map(u64::to_string).collect();
            format!("\"{}\":[{}]", json_escape(k), s.join(","))
        })
        .collect::<Vec<_>>()
        .join(",")
}

fn arr(v: &[String]) -> String {
    v.iter()
        .map(|s| format!("\"{}\"", json_escape(s)))
        .collect::<Vec<_>>()
        .join(",")
}

#[cfg(test)]
#[path = "receipt_report_tests.rs"]
mod tests;
