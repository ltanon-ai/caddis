//! netpipe.rs — the pipe-to-shell steer (DESTRUCTIVE-1, gemini ruling).
//!
//! `curl … | bash` (and the wget variant) STEERS, never denies: legit
//! installer patterns are a real everyday case and a hard deny there would
//! cost the tool its credibility. The finding carries a DOMAIN TRUSTLIST —
//! the known rustup / Homebrew / bun bootstrap patterns are named as such —
//! while everything else steers showing the EXACT URL, so the reader can
//! judge the source with the evidence in hand.
//!
//! Local `cat file | bash` is deliberately OUTSIDE this law: nothing flows
//! in from the net, and the ruling is net.pipe-to-shell.

use super::Finding;
use crate::checks::cmdline::segments_detailed;

/// Shells a pipe can hand remote text to.
const SHELLS: &[&str] = &["sh", "bash", "zsh"];

/// Fetchers whose stdout the pipe carries.
const FETCHERS: &[&str] = &["curl", "wget"];

/// Known-good installer sources — (needle, human name). A needle matches
/// inside the URL; the name is what the finding shows.
const TRUSTED: &[(&str, &str)] = &[
    ("sh.rustup.rs", "rustup"),
    ("raw.githubusercontent.com/homebrew/", "Homebrew"),
    ("brew.sh", "Homebrew"),
    ("bun.sh", "bun"),
];

/// The base name of a command token, path-stripped both separator ways.
fn base_of(token: &str) -> &str {
    token.rsplit(['/', '\\']).next().unwrap_or(token)
}

/// Does this pipe stage fetch from the net?
fn is_fetch(tokens: &[String]) -> bool {
    tokens
        .first()
        .map(|t| FETCHERS.contains(&base_of(t)))
        .unwrap_or(false)
}

fn url_of(tokens: &[String]) -> String {
    for t in tokens.iter().skip(1) {
        if t.starts_with("http://") || t.starts_with("https://") {
            return t.clone();
        }
    }
    tokens
        .iter()
        .skip(1)
        .find(|t| !t.starts_with('-'))
        .cloned()
        .unwrap_or_default()
}

/// The trusted name for a URL, if any needle matches (case-folded — hosts
/// are case-insensitive).
fn trusted_for(url: &str) -> Option<&'static str> {
    let lowered = url.to_ascii_lowercase();
    TRUSTED
        .iter()
        .find(|(needle, _)| lowered.contains(needle))
        .map(|(_, name)| *name)
}

/// SOFT: remote text piped straight into a shell.
pub fn pipe_to_shell(command: &str) -> Finding {
    let stages = segments_detailed(command);
    for (i, stage) in stages.iter().enumerate() {
        if !is_fetch(&stage.tokens) {
            continue;
        }
        let url = url_of(&stage.tokens);
        let piped = stages
            .iter()
            .skip(i + 1)
            .take_while(|s| s.sep_before.as_deref() == Some("|"));
        for next in piped {
            let into_shell = next
                .tokens
                .first()
                .map(|t| SHELLS.contains(&base_of(t)))
                .unwrap_or(false);
            if !into_shell {
                continue;
            }
            return match trusted_for(&url) {
                Some(name) => Some(format!(
                    "`{url}` is the {name} bootstrap pattern — known, and still piped straight \
                     into a shell. Fine for a trusted morning; pin a revision when it matters."
                )),
                None => Some(format!(
                    "`{url}` is piped straight into a shell with no trust on record. Read it, \
                     pin a revision, or download-then-inspect — the URL is the whole decision."
                )),
            };
        }
    }
    None
}
