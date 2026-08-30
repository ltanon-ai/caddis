//! page_report_tally.rs — observe-log tally, split from page_report.rs
//! at the 280-line cap. Zero-dep substring readers for OUR log lines.

/// Minimal JSON field readers for OUR log lines only (zero-dep): the
/// observe nerve writes them, so the shape is ours to parse.
pub(crate) fn json_str<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let pat = format!("\"{key}\":\"");
    let start = line.find(&pat)? + pat.len();
    let end = line[start..].find('"')? + start;
    Some(&line[start..end])
}

pub(crate) fn json_num(line: &str, key: &str) -> Option<u64> {
    let pat = format!("\"{key}\":");
    let start = line.find(&pat)? + pat.len();
    let digits: String = line[start..]
        .chars()
        .take_while(|c| c.is_ascii_digit())
        .collect();
    digits.parse().ok()
}

#[derive(Default)]
pub(crate) struct Tally {
    pub(crate) events: u64,
    pub(crate) fails: u64,
    pub(crate) before: u64,
    pub(crate) auto: u64,
    pub(crate) last_stored: Option<u64>,
    pub(crate) last_sent: Option<u64>,
    pub(crate) last_pct: Option<u64>,
    pub(crate) last_window: Option<u64>,
    pub(crate) last_n: Option<u64>,
    pub(crate) last_chars: Option<u64>,
    pub(crate) last_largest: Option<u64>,
    pub(crate) last_custom: Option<u64>,
    pub(crate) last_user: Option<u64>,
    pub(crate) last_assistant: Option<u64>,
    pub(crate) last_tool_result: Option<u64>,
    pub(crate) last_page_mode: Option<bool>,
    pub(crate) last_n_stubbed: Option<u64>,
    pub(crate) last_n_evicted: Option<u64>,
    pub(crate) last_user_chars: Option<u64>,
    pub(crate) last_assistant_chars: Option<u64>,
    pub(crate) last_tool_result_chars: Option<u64>,
    pub(crate) last_usage: Option<String>,
    pub(crate) n_fault: u64,
    pub(crate) n_ref: u64,
    pub(crate) last_recovery_ms: Option<u64>,
}

pub(crate) fn tally_line(line: &str, t: &mut Tally) {
    match json_str(line, "kind") {
        Some("context") => {
            t.events += 1;
            if line.contains("\"parse_ok\":false") {
                t.fails += 1;
            }
            t.last_stored = json_num(line, "stored_tokens").or(t.last_stored);
            t.last_sent = json_num(line, "sent_est_tokens").or(t.last_sent);
            t.last_pct = json_num(line, "stored_pct").or(t.last_pct);
            t.last_window = json_num(line, "stored_window").or(t.last_window);
            t.last_n = json_num(line, "n_messages").or(t.last_n);
            t.last_chars = json_num(line, "chars").or(t.last_chars);
            t.last_largest = json_num(line, "largest_tool_result_chars").or(t.last_largest);
            t.last_custom = json_num(line, "custom").or(t.last_custom);
            t.last_user = json_num(line, "user").or(t.last_user);
            t.last_user_chars = json_num(line, "user_chars").or(t.last_user_chars);
            t.last_assistant_chars = json_num(line, "assistant_chars").or(t.last_assistant_chars);
            t.last_tool_result_chars =
                json_num(line, "toolResult_chars").or(t.last_tool_result_chars);
            t.last_assistant = json_num(line, "assistant").or(t.last_assistant);
            t.last_tool_result = json_num(line, "toolResult").or(t.last_tool_result);
            if line.contains("\"page_mode\":true") {
                t.last_page_mode = Some(true);
            } else if line.contains("\"page_mode\":false") {
                t.last_page_mode = Some(false);
            }
            t.last_n_stubbed = json_num(line, "n_stubbed").or(t.last_n_stubbed);
        }
        Some("compact_before") => t.before += 1,
        Some("compact_auto_start") => t.auto += 1,
        Some("project") => t.last_n_evicted = json_num(line, "n_evicted").or(t.last_n_evicted),
        Some("message_end") => {
            if line.contains("\"usage\":{") {
                t.last_usage = Some(line.to_string());
            }
        }
        Some("fault") => {
            t.n_fault += 1;
            t.last_recovery_ms = json_num(line, "recovery_ms").or(t.last_recovery_ms);
        }
        Some("ref") => {
            t.n_ref += 1;
            t.last_recovery_ms = json_num(line, "recovery_ms").or(t.last_recovery_ms);
        }
        _ => {}
    }
}
