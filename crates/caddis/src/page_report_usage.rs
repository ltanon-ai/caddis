//! page_report_usage.rs — CARD-0213 split from page_report.rs (280-line cap).

pub(crate) fn print_usage(last_usage: &Option<String>) {
    let Some(u) = last_usage else { return };
    let Some(obj) = usage_slice(u) else { return };
    for key in [
        "input",
        "cacheRead",
        "cacheWrite",
        "output",
        "reasoningTokens",
    ] {
        if let Some(v) = json_num_depth1(obj, key) {
            println!("{key}={v}");
        }
    }
}

fn usage_slice(line: &str) -> Option<&str> {
    let needle = "\"usage\":{";
    let start = line.find(needle)?;
    let from = start + needle.len() - 1;
    let b = line.as_bytes();
    let mut depth = 0u32;
    for (off, &c) in b[from..].iter().enumerate() {
        match c {
            b'{' => depth += 1,
            b'}' => {
                depth -= 1;
                if depth == 0 {
                    return Some(&line[from..=from + off]);
                }
            }
            _ => {}
        }
    }
    None
}

fn json_num_depth1(obj: &str, key: &str) -> Option<u64> {
    let pat = format!("\"{key}\":");
    let b = obj.as_bytes();
    let mut depth = 0i32;
    let mut i = 0;
    while i < obj.len() {
        match b.get(i) {
            Some(&b'{') => depth += 1,
            Some(&b'}') => depth -= 1,
            _ if depth == 1 && obj[i..].starts_with(&pat) => {
                let s = i + pat.len();
                let digits: String = obj[s..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                return digits.parse().ok();
            }
            _ => {}
        }
        i += 1;
    }
    None
}
