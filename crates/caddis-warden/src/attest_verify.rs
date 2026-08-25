//! attest_verify.rs — rendering a bundle, and re-checking one against the
//! ledger (CARD-0114), split from `attest.rs` under the 280-line law.
//!
//! ⛔ A VERIFIER THAT CANNOT FAIL IS NOT A VERIFIER. `--verify` recomputes every
//! counted claim from the ledger and compares it to what the bundle asserts;
//! any mismatch is CONTRADICTED and the process exits non-zero. The tamper
//! cases are pinned by tests, because a verifier nobody has seen go red is
//! `assert(true == true)` with extra steps.

use crate::attest::{Bundle, LIMITS};
use crate::wire::json_escape;

/// The `path` field of a `tag|command|path|why` body, split from the RIGHT so
/// pipes inside the command survive.
pub fn row_path(body: &str) -> String {
    body.rsplit_once('|')
        .and_then(|(head, _why)| head.rsplit_once('|').map(|(_, p)| p.to_string()))
        .unwrap_or_default()
}

pub fn render_text(b: &Bundle) -> String {
    let mut s = format!(
        "attest: {}\n  card file   : {} (hash {})\n  caller      : {}\n  \
         window      : ledger rows {}..{} (physical position, never seq)\n  \
         declared    : blast {}, allowlist {}\n  verdicts    : allow {} steer {} deny {}\n  \
         files       : {} distinct written",
        b.card_id,
        b.card_path,
        b.card_hash,
        b.from,
        b.opened_at_row,
        b.closed_at_row,
        b.blast,
        if b.allowlist.is_empty() {
            "(none declared — a v1 card bounds nothing)".to_string()
        } else {
            b.allowlist.join(", ")
        },
        b.allow,
        b.steer,
        b.deny,
        b.files.len()
    );
    if !b.card_readable {
        // ⛔ NOT "none". The one case where nothing could be checked is the case
        // where a reader most needs to be told so (pre-push review, finding #6).
        s.push_str(
            "\n  OUTSIDE     : UNKNOWN — the card file could not be read at attest time, \
             so NOTHING was checked against its allowlist",
        );
    } else if b.outside.is_empty() {
        s.push_str("\n  OUTSIDE     : none");
    } else {
        s.push_str(&format!(
            "\n  OUTSIDE     : {} file(s) written outside the declared allowlist:",
            b.outside.len()
        ));
        for p in &b.outside {
            s.push_str(&format!("\n                {p}"));
        }
    }
    s.push_str(&format!(
        "\n  RED-TEST    : {}",
        if b.red_test_seen {
            "a matching command was ATTEMPTED in the window (not proof it passed)"
        } else {
            "no matching command seen in the window"
        }
    ));
    if !b.laws.is_empty() {
        let l: Vec<String> = b.laws.iter().map(|(k, v)| format!("{k}={v}")).collect();
        s.push_str(&format!("\n  laws fired  : {}", l.join("  ")));
    }
    s.push_str(&format!(
        "\n  unreadable  : {} ledger line(s)\n\nLIMITS OF THIS BUNDLE:",
        b.unreadable
    ));
    for l in LIMITS {
        s.push_str(&format!("\n  - {l}"));
    }
    s
}

pub fn render_json(b: &Bundle) -> String {
    let files: Vec<String> = b
        .files
        .iter()
        .map(|(k, v)| format!("\"{}\":{}", json_escape(k), v))
        .collect();
    let laws: Vec<String> = b
        .laws
        .iter()
        .map(|(k, v)| format!("\"{}\":{}", json_escape(k), v))
        .collect();
    let arr = |v: &[String]| {
        v.iter()
            .map(|s| format!("\"{}\"", json_escape(s)))
            .collect::<Vec<_>>()
            .join(",")
    };
    format!(
        "{{\"card_id\":\"{}\",\"card_path\":\"{}\",\"card_hash\":\"{}\",\"from\":\"{}\",\
         \"opened_at_row\":{},\"closed_at_row\":{},\"blast\":{},\"allowlist\":[{}],\
         \"allow\":{},\"steer\":{},\"deny\":{},\"files\":{{{}}},\
         \"files_outside_allowlist\":[{}],\"card_readable\":{},\
         \"red_test_attempted\":{},\"laws\":{{{}}},\
         \"unreadable\":{},\"limits\":[{}]}}",
        json_escape(&b.card_id),
        json_escape(&b.card_path),
        json_escape(&b.card_hash),
        json_escape(&b.from),
        b.opened_at_row,
        b.closed_at_row,
        b.blast,
        arr(&b.allowlist),
        b.allow,
        b.steer,
        b.deny,
        files.join(","),
        arr(&b.outside),
        b.card_readable,
        b.red_test_seen,
        laws.join(","),
        b.unreadable,
        arr(&LIMITS.iter().map(|s| s.to_string()).collect::<Vec<_>>())
    )
}

/// A number field out of a bundle, by name. Hand-rolled under the crate's
/// zero-dependency law, same as everything else that touches JSON here.
fn num(json: &str, key: &str) -> Option<u64> {
    let at = json.find(&format!("\"{key}\":"))? + key.len() + 3;
    json[at..]
        .chars()
        .take_while(char::is_ascii_digit)
        .collect::<String>()
        .parse()
        .ok()
}

fn text_field(json: &str, key: &str) -> Option<String> {
    let at = json.find(&format!("\"{key}\":\""))? + key.len() + 4;
    Some(json[at..].split('"').next()?.to_string())
}

/// How many entries a JSON array field holds. Enough to catch the tamper that
/// matters most — emptying `files_outside_allowlist`.
fn arr_len(json: &str, key: &str) -> Option<usize> {
    let at = json.find(&format!("\"{key}\":["))? + key.len() + 4;
    let body = json[at..].split(']').next()?;
    if body.trim().is_empty() {
        return Some(0);
    }
    Some(body.matches('"').count() / 2)
}

/// How many keys a JSON OBJECT field holds.
///
/// ⚠ SEPARATE FROM `arr_len` BECAUSE THE TWO SHAPES ARE NOT INTERCHANGEABLE.
/// Reading `"files":{...}` with the array reader returns None, which the
/// comparison renders as `(absent)` and reports CONTRADICTED — a true bundle
/// failing its own verification. Found exactly that way: the verifier caught
/// this bug in the verifier.
fn obj_len(json: &str, key: &str) -> Option<usize> {
    let at = json.find(&format!("\"{key}\":{{"))? + key.len() + 4;
    let body = json[at..].split('}').next()?;
    if body.trim().is_empty() {
        return Some(0);
    }
    Some(body.matches(':').count())
}

struct Claim {
    name: &'static str,
    claimed: String,
    actual: String,
}

impl Claim {
    fn ok(&self) -> bool {
        self.claimed == self.actual
    }
}

pub fn verify(bundle_path: &str) -> i32 {
    let json = match std::fs::read_to_string(bundle_path) {
        Ok(t) => t,
        Err(e) => {
            eprintln!("attest --verify: cannot read {bundle_path}: {e}");
            return 2;
        }
    };
    let Some(card_id) = text_field(&json, "card_id") else {
        eprintln!("attest --verify: {bundle_path} carries no card_id; it is not a bundle");
        return 2;
    };
    let Some(text) = crate::propose::read_ledger("attest --verify") else {
        return 2;
    };
    let fresh = match crate::attest::build(&text, &card_id) {
        Ok(b) => b,
        Err(why) => {
            eprintln!("attest --verify: {why}");
            return 2;
        }
    };
    let claims = compare(&json, &fresh);
    let bad = claims.iter().filter(|c| !c.ok()).count();
    println!("attest --verify: {card_id} against the live ledger");
    for c in &claims {
        println!(
            "  {} {:<24} bundle={} ledger={}",
            if c.ok() {
                "CONFIRMED  "
            } else {
                "CONTRADICTED"
            },
            c.name,
            c.claimed,
            c.actual
        );
    }
    if bad == 0 {
        println!("\nALL {} CLAIM(S) CONFIRMED.", claims.len());
        return 0;
    }
    // Non-zero, loudly. A bundle whose numbers do not survive a recomputation
    // is worse than no bundle: it is a claim wearing evidence's clothes.
    println!("\n{bad} CLAIM(S) CONTRADICTED — this bundle does not match the ledger.");
    1
}

fn compare(json: &str, fresh: &Bundle) -> Vec<Claim> {
    let n = |key: &str, actual: u64| Claim {
        name: Box::leak(key.to_string().into_boxed_str()),
        claimed: num(json, key).map_or("(absent)".into(), |v| v.to_string()),
        actual: actual.to_string(),
    };
    vec![
        n("allow", fresh.allow),
        n("steer", fresh.steer),
        n("deny", fresh.deny),
        n("opened_at_row", fresh.opened_at_row as u64),
        n("closed_at_row", fresh.closed_at_row as u64),
        Claim {
            name: "card_hash",
            claimed: text_field(json, "card_hash").unwrap_or_else(|| "(absent)".into()),
            actual: fresh.card_hash.clone(),
        },
        Claim {
            name: "from",
            claimed: text_field(json, "from").unwrap_or_else(|| "(absent)".into()),
            actual: fresh.from.clone(),
        },
        Claim {
            name: "files_outside_count",
            claimed: arr_len(json, "files_outside_allowlist")
                .map_or("(absent)".into(), |v| v.to_string()),
            actual: fresh.outside.len().to_string(),
        },
        Claim {
            name: "files_distinct",
            claimed: obj_len(json, "files").map_or("(absent)".into(), |v| v.to_string()),
            actual: fresh.files.len().to_string(),
        },
    ]
}

#[cfg(test)]
#[path = "attest_verify_tests.rs"]
mod tests;
