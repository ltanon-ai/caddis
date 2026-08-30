//! deja_vu.rs — CARD-0245 RED-first. Cross-session attention replay.
//!
//! Every session's observe.jsonl records what was injected into context
//! (the `facts` array on each `kind:"context"` event) and when the model
//! referenced one of those facts (the `kind:"cite"` event with `fact`).
//! The organ must scan a project dir of observe trails, aggregate those
//! signals into an `AttentionMap`, and surface the facts nobody cited.
//! Those are the A/B candidates for the host to strip.
//!
//! Today's reality: nothing in the workspace opens two observe.jsonl
//! files in one process. The RED pins a dead fact (zero citations across
//! a 3-session trail) as a discoverable candidate — before this card the
//! test could not even NAME it, because the type and the reader do not
//! exist.

use std::fs;
use std::path::PathBuf;

use caddis_organs::deja_vu::{build, dead_weight, AttentionMap, FactKey, FactStats};

/// Seed a single observe.jsonl with one context event and zero citations.
/// One fact is "alive" (the model cited it), one is "dead" (zero cites).
fn write_session(dir: &std::path::Path, name: &str, lines: &[&str]) -> PathBuf {
    fs::create_dir_all(dir).unwrap();
    let p = dir.join(format!("{name}.observe.jsonl"));
    fs::write(&p, lines.join("\n") + "\n").unwrap();
    p
}

/// RED: a 3-session trail where one fact "alive" gets cited and one
/// "dead" never does. `dead_weight` must return the dead fact as a
/// candidate. Today the test cannot even compile — the type and the
/// reader do not exist.
#[test]
fn dead_fact_surfaces_across_three_sessions() {
    let tmp = std::env::temp_dir().join(format!("caddis-dejavu-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);

    let alive: FactKey = "sha256:alive".to_string();
    let dead: FactKey = "sha256:dead".to_string();

    // Three sessions, all inject the same two facts; only `alive` gets cited.
    let lines_alive = |cite: bool| -> String {
        let mut s = String::new();
        s.push_str(&format!(
            "{{\"kind\":\"context\",\"parse_ok\":true,\"stored_tokens\":1000,\"sent_est_tokens\":1000,\"facts\":[\"{alive}\",\"{dead}\"]}}\n"
        ));
        s.push_str("{\"kind\":\"message_end\",\"usage\":{\"tokens\":42}}\n");
        if cite {
            s.push_str(&format!("{{\"kind\":\"cite\",\"fact\":\"{alive}\"}}\n"));
        }
        s
    };

    let p1 = write_session(&tmp.join("s1"), "alpha", &[&lines_alive(true)]);
    let p2 = write_session(&tmp.join("s2"), "beta", &[&lines_alive(true)]);
    let p3 = write_session(&tmp.join("s3"), "gamma", &[&lines_alive(false)]);

    let trails = vec![p1, p2, p3];
    let map: AttentionMap = build(&trails);

    // Sanity: both facts are seen, alive is cited, dead is not.
    let alive_stats: &FactStats = map.facts.get(&alive).expect("alive present");
    assert_eq!(alive_stats.sessions_seen, 3, "alive seen in all 3 sessions");
    assert!(alive_stats.citations >= 2, "alive cited in s1 and s2");
    assert!(alive_stats.tokens_burned > 0, "alive burned tokens");

    let dead_stats: &FactStats = map.facts.get(&dead).expect("dead present");
    assert_eq!(dead_stats.sessions_seen, 3, "dead seen in all 3 sessions");
    assert_eq!(dead_stats.citations, 0, "dead NEVER cited");

    // THE assertion: dead_weight surfaces the dead fact across a
    // generous window (u64::MAX == include all trails regardless of age).
    let candidates = dead_weight(&map, u64::MAX);
    assert!(
        candidates.contains(&dead),
        "dead fact must be an A/B candidate; got {:?}",
        candidates
    );
    assert!(
        !candidates.contains(&alive),
        "cited fact must NOT be a candidate; got {:?}",
        candidates
    );

    let _ = fs::remove_dir_all(&tmp);
}

/// RED: a missing trail (path that does not exist) must NOT panic — the
/// reader is fail-safe. Empty map is fine.
#[test]
fn missing_trail_is_fail_safe() {
    let bogus = PathBuf::from("/no/such/dir/never.observe.jsonl");
    let map = build(&[bogus]);
    assert!(map.facts.is_empty(), "missing trail -> empty map");
    assert!(dead_weight(&map, u64::MAX).is_empty());
}

/// RED: a session trail with only citation events (no `facts` array)
/// must still tally citations. Today the type doesn't exist.
#[test]
fn cite_without_context_still_counts() {
    let tmp = std::env::temp_dir().join(format!("caddis-dejavu-citeonly-{}", std::process::id()));
    let _ = fs::remove_dir_all(&tmp);
    let p = write_session(
        &tmp.join("citeonly"),
        "zeta",
        &["{\"kind\":\"cite\",\"fact\":\"sha256:z\"}"],
    );
    let map = build(&[p]);
    let z = map.facts.get("sha256:z").expect("z present");
    assert_eq!(z.citations, 1);
    assert!(!dead_weight(&map, u64::MAX).contains(&"sha256:z".to_string()));
    let _ = fs::remove_dir_all(&tmp);
}
