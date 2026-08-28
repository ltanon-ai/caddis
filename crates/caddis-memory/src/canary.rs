//! canary.rs — the golden-query canary hop for the recall organ.
//!
//! Organs law (caddis-organs canary precedent): GREEN = the chain provably
//! works right now; RED = it provably does not (the HOST halts — this organ
//! reports, never kills); DEGRADED = the external lane is unreachable, which
//! NEVER halts. A machine without qmd (public CI) gets DEGRADED, not RED —
//! exactly like the source canary's absent-model-lane hop.

use crate::recall::{MemoryConfig, Recall, RecallError};

#[derive(Debug, Clone, PartialEq)]
pub enum Verdict {
    /// The golden query ran and returned the expected document.
    Green { hits: usize, ms: u128 },
    /// The lane ran and provably failed: the golden doc is missing from the
    /// results, or the lane itself failed (timeout, nonzero, unparseable).
    Red(String),
    /// qmd is not usable on this machine right now (spawn failure only).
    Degraded(String),
}

/// One golden probe: a query whose expected answer is known in advance.
#[derive(Debug, Clone)]
pub struct Golden {
    /// 0 = fast lane (`search`), 1 = deep lane (`query`).
    pub deep: bool,
    pub query: String,
    /// Substring that must appear in some hit's `file` (e.g. a fixture or a
    /// known memory doc name). Chosen to be unique per probe.
    pub expect_file_contains: String,
}

impl Golden {
    pub fn fast(query: &str, expect_file_contains: &str) -> Self {
        Golden {
            deep: false,
            query: query.to_string(),
            expect_file_contains: expect_file_contains.to_string(),
        }
    }

    pub fn deep(query: &str, expect_file_contains: &str) -> Self {
        Golden {
            deep: true,
            query: query.to_string(),
            expect_file_contains: expect_file_contains.to_string(),
        }
    }
}

/// Run one golden probe against the configured index (real lane).
pub fn run(config: &MemoryConfig, golden: &Golden) -> Verdict {
    let mut recall = Recall::new(config.clone());
    run_on(&mut recall, golden)
}

/// Run one golden probe through an existing recall handle (test seam).
pub fn run_on<R: crate::exec::Runner>(recall: &mut Recall<R>, golden: &Golden) -> Verdict {
    let attempt = if golden.deep {
        recall.query(&golden.query)
    } else {
        recall.search(&golden.query)
    };
    match attempt {
        Err(RecallError::Spawn(msg)) => Verdict::Degraded(msg),
        Err(other) => Verdict::Red(format!("golden lane failed: {other}")),
        Ok((hits, report)) => {
            let found = hits
                .iter()
                .any(|h| h.file.contains(&golden.expect_file_contains));
            if found {
                Verdict::Green {
                    hits: hits.len(),
                    ms: report.duration.as_millis(),
                }
            } else {
                Verdict::Red(format!(
                    "golden query {:?} returned {} hit(s), none containing {:?}",
                    golden.query,
                    hits.len(),
                    golden.expect_file_contains
                ))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::exec::testing::FakeRunner;
    use crate::recall::Recall;

    fn cfg() -> MemoryConfig {
        MemoryConfig {
            launcher: vec!["node".into(), "qmd".into()],
            workdir: None,
            ..MemoryConfig::default()
        }
    }

    #[test]
    fn green_when_expected_doc_present() {
        let mut fake = FakeRunner::default();
        fake.on(
            "search",
            FakeRunner::ok_json(
                "search",
                r##"[{"docid":"#1","file":"qmd://docs/golden.md","score":0.9}]"##,
            ),
        );
        let mut recall = Recall::with_runner(cfg(), fake);
        let verdict = run_on(
            &mut recall,
            &Golden::fast("golden needle", "docs/golden.md"),
        );
        assert!(
            matches!(verdict, Verdict::Green { hits: 1, .. }),
            "got {verdict:?}"
        );
    }

    #[test]
    fn red_when_doc_missing() {
        let mut fake = FakeRunner::default();
        fake.on(
            "search",
            FakeRunner::ok_json("search", r##"[{"docid":"#1","file":"qmd://other.md"}]"##),
        );
        let mut recall = Recall::with_runner(cfg(), fake);
        let verdict = run_on(
            &mut recall,
            &Golden::fast("golden needle", "docs/golden.md"),
        );
        match verdict {
            Verdict::Red(msg) => assert!(msg.contains("none containing"), "got {msg}"),
            other => panic!("expected Red, got {other:?}"),
        }
    }

    #[test]
    fn spawn_failure_is_degraded_never_red() {
        let mut fake = FakeRunner::default();
        let mut dead = FakeRunner::ok_json("search", "[]");
        dead.code = None;
        dead.stderr = "spawn failed: node not found".into();
        fake.on("search", dead);
        let mut recall = Recall::with_runner(cfg(), fake);
        let verdict = run_on(&mut recall, &Golden::fast("x", "y"));
        assert!(matches!(verdict, Verdict::Degraded(_)), "got {verdict:?}");
    }

    #[test]
    fn timeout_is_red_not_degraded() {
        let mut fake = FakeRunner::default();
        let mut killed = FakeRunner::ok_json("query", "[]");
        killed.timed_out = true;
        killed.code = None;
        fake.on("query", killed);
        let mut recall = Recall::with_runner(cfg(), fake);
        let verdict = run_on(&mut recall, &Golden::deep("x", "y"));
        match verdict {
            Verdict::Red(msg) => assert!(msg.contains("killed at lane budget"), "got {msg}"),
            other => panic!("expected Red, got {other:?}"),
        }
    }
}
