//! detect.rs — the F-A1 language-detection ladder (T-35 verdict, rung 6 P1).
//!
//! Ladder per sentence segment, first match wins:
//!
//! - **L1 diacritic scan** — text carries ąčęėįšųūž → LT, confidence 1.0.
//!   Marked Lithuanian is unambiguous and free to detect.
//! - **L2 offline trigram** — unmarked text goes to [`crate::trigram`].
//!   Ambiguous verdicts tie-break **LT** (F-A1: uncertain LT/EN → route LT —
//!   Leonas reading EN is accented-but-correct; ryan reading LT is the
//!   banned garbage). No-letters segments (numbers, "ok"-class) fall to L0.
//! - **L0 declared default** — the label's declared language; with no
//!   declaration the ambiguity rule answers LT.
//!
//! Hard latency cap (30ms default): the ladder measures cumulative elapsed
//! time (post-cache) and any segment that would start L2 after the cap is
//! spent answers via cap-fallback (L0/LT) with `over_cap: true` — eager,
//! capped, telemetered, exactly as the quorum carried it. Inputs beyond
//! [`DetectOptions::max_bytes`] are analyzed on a prefix and marked
//! `truncated`.
//!
//! Text-hash cache: final verdicts are cached by FNV-1a of (text, declared);
//! a repeated utterance replays in O(1) with `from_cache: true` (the WAV
//! cache keys off the same hash later, so the caches cannot disagree).

use crate::lang::{split_sentences, Lang};
use crate::trigram;
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// The layer that decided a segment. Order of authority: L1 > L2 > L0.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layer {
    /// Label-declared default (or the LT ambiguity rule) decided.
    L0Declared,
    /// Diacritic scan decided: LT.
    L1Diacritic,
    /// Offline trigram classifier decided (including LT tie-breaks).
    L2Trigram,
    /// The latency cap fired before L2 could run — fallback answer.
    CapFallback,
}

/// One segment's decision + per-decision telemetry (F-A1: language picked,
/// confidence, layer that decided, cap behavior).
#[derive(Debug, Clone, PartialEq)]
pub struct Decision {
    pub lang: Lang,
    pub layer: Layer,
    /// 0.0..=1.0; 1.0 for L1; the trigram margin for L2; 0.0 for fallbacks.
    pub confidence: f64,
    /// True when the ambiguity tie-break (→ LT) fired inside L2.
    pub tie_break: bool,
    /// True when this segment answered via cap-fallback.
    pub over_cap: bool,
}

/// A segment with its text and decision.
#[derive(Debug, Clone, PartialEq)]
pub struct Segment {
    pub text: String,
    pub decision: Decision,
}

/// Whole-utterance result.
#[derive(Debug, Clone, PartialEq)]
pub struct Utterance {
    pub segments: Vec<Segment>,
    /// True when the segments did not all agree on one language.
    pub mixed: bool,
    /// True when this exact (text, declared) verdict came from the cache.
    pub from_cache: bool,
    /// True when the input exceeded `max_bytes` and a prefix was analyzed.
    pub truncated: bool,
}

/// Cache telemetry counters (mirrored into soak reports — R-C).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CacheStats {
    pub hits: u64,
    pub misses: u64,
}

/// The LT diacritic set (both cases) that arms L1.
const LT_DIACRITICS: &str = "ąčęėįšųūžĄČĘĖĮŠŲŪŽ";

#[derive(Debug, Clone)]
pub struct DetectOptions {
    /// Hard cumulative budget for one `detect` call. Default 30ms (F-A1).
    pub cap: Duration,
    /// Inputs longer than this are analyzed on a prefix. Default 16 KiB —
    /// utterances are sentences, not books.
    pub max_bytes: usize,
    /// Cache capacity; at capacity the cache clears wholesale (simple,
    /// deterministic, and a cleared cache only costs reclassification).
    pub cache_capacity: usize,
}

impl Default for DetectOptions {
    fn default() -> Self {
        DetectOptions {
            cap: Duration::from_millis(30),
            max_bytes: 16 * 1024,
            cache_capacity: 4096,
        }
    }
}

/// The ladder. Owns the verdict cache and its stats.
pub struct Detector {
    opts: DetectOptions,
    cache: HashMap<u64, CachedVerdict>,
    pub stats: CacheStats,
}

#[derive(Debug, Clone)]
struct CachedVerdict {
    segments: Vec<Decision>,
    mixed: bool,
    truncated: bool,
}

impl Detector {
    pub fn new(opts: DetectOptions) -> Detector {
        Detector {
            opts,
            cache: HashMap::new(),
            stats: CacheStats::default(),
        }
    }

    /// Run the ladder over an utterance. `declared` is the label's declared
    /// default language (L0) — `None` means undeclared, and the ambiguity
    /// rule (LT) answers undecidable segments.
    pub fn detect(&mut self, text: &str, declared: Option<Lang>) -> Utterance {
        let key = fnv1a(text, declared);
        if let Some(v) = self.cache.get(&key) {
            self.stats.hits += 1;
            let segments = split_sentences(text)
                .into_iter()
                .zip(v.segments.iter().cloned())
                .map(|(t, d)| Segment { text: t.to_string(), decision: d })
                .collect();
            return Utterance {
                segments,
                mixed: v.mixed,
                from_cache: true,
                truncated: v.truncated,
            };
        }
        self.stats.misses += 1;

        let truncated = text.len() > self.opts.max_bytes;
        let analyzed = if truncated { &text[..truncation_point(text, self.opts.max_bytes)] } else { text };
        let started = Instant::now();
        let sentences = split_sentences(analyzed);

        let mut segments = Vec::with_capacity(sentences.len());
        for s in sentences {
            let over_cap = started.elapsed() > self.opts.cap;
            let decision = self.decide_segment(s, declared, over_cap);
            segments.push(Segment {
                text: s.to_string(),
                decision,
            });
        }
        if segments.is_empty() {
            // Empty or boundary-only input: one L0 answer keeps the shape.
            segments.push(Segment {
                text: String::new(),
                decision: self.decide_segment("", declared, false),
            });
        }
        let mixed = {
            let mut langs = segments.iter().map(|s| s.decision.lang);
            let first = langs.next();
            langs.any(|l| Some(l) != first)
        };

        let cached = CachedVerdict {
            segments: segments.iter().map(|s| s.decision.clone()).collect(),
            mixed,
            truncated,
        };
        if self.cache.len() >= self.opts.cache_capacity {
            self.cache.clear();
        }
        self.cache.insert(key, cached);

        Utterance {
            segments,
            mixed,
            from_cache: false,
            truncated,
        }
    }

    fn decide_segment(&self, s: &str, declared: Option<Lang>, over_cap: bool) -> Decision {
        if over_cap {
            return Decision {
                lang: declared.unwrap_or(Lang::Lt),
                layer: Layer::CapFallback,
                confidence: 0.0,
                tie_break: false,
                over_cap: true,
            };
        }
        // L1: marked Lithuanian decides immediately.
        if s.chars().any(|c| LT_DIACRITICS.contains(c)) {
            return Decision {
                lang: Lang::Lt,
                layer: Layer::L1Diacritic,
                confidence: 1.0,
                tie_break: false,
                over_cap: false,
            };
        }
        // L2: offline trigram for unmarked text.
        if let Some(v) = trigram::classify(s) {
            return Decision {
                lang: v.lang,
                layer: Layer::L2Trigram,
                confidence: v.confidence,
                tie_break: v.ambiguous, // ambiguous → classifier already answered LT
                over_cap: false,
            };
        }
        // L0: no signal at all — declared default, else the LT ambiguity rule.
        Decision {
            lang: declared.unwrap_or(Lang::Lt),
            layer: Layer::L0Declared,
            confidence: 0.0,
            tie_break: declared.is_none(),
            over_cap: false,
        }
    }
}

/// FNV-1a over text + declared language. Same key the WAV cache will use,
/// so language verdicts and audio can never disagree about identity.
pub fn fnv1a(text: &str, declared: Option<Lang>) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for b in text.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    for b in declared.map(|l| l.as_str()).unwrap_or("\0").as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

/// Largest char-boundary ≤ max_bytes.
fn truncation_point(text: &str, max_bytes: usize) -> usize {
    let mut i = text.len().min(max_bytes);
    while i > 0 && !text.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;

    fn det() -> Detector {
        Detector::new(DetectOptions::default())
    }

    #[test]
    fn marked_lt_wins_even_when_declared_en() {
        let u = det().detect("labai ačiū už pagalbą", Some(Lang::En));
        assert_eq!(u.segments[0].decision.layer, Layer::L1Diacritic);
        assert_eq!(u.segments[0].decision.lang, Lang::Lt);
        assert!(!u.mixed);
    }

    #[test]
    fn unmarked_lt_reaches_l2() {
        let u = det().detect("labas vakaras mano mielas drauge", None);
        assert_eq!(u.segments[0].decision.layer, Layer::L2Trigram);
        assert_eq!(u.segments[0].decision.lang, Lang::Lt);
    }

    #[test]
    fn english_text_is_en() {
        let u = det().detect("good evening my dear friend", Some(Lang::Lt));
        assert_eq!(u.segments[0].decision.lang, Lang::En);
    }

    #[test]
    fn no_letters_falls_to_declared_default() {
        let u = det().detect("123 456 789", Some(Lang::En));
        assert_eq!(u.segments[0].decision.layer, Layer::L0Declared);
        assert_eq!(u.segments[0].decision.lang, Lang::En);
    }

    #[test]
    fn undeclared_undecidable_answers_lt() {
        let u = det().detect("123 456", None);
        assert_eq!(u.segments[0].decision.lang, Lang::Lt);
        assert!(u.segments[0].decision.tie_break);
    }

    #[test]
    fn mixed_utterance_is_marked_mixed() {
        let u = det().detect("Labas, drauge. Good evening my friend. Ačiū tau.", None);
        assert!(u.mixed, "segments: {:?}", u.segments);
        let langs: Vec<Lang> = u.segments.iter().map(|s| s.decision.lang).collect();
        assert!(langs.contains(&Lang::Lt) && langs.contains(&Lang::En));
    }

    #[test]
    fn cache_replays_verdict() {
        let mut d = det();
        let first = d.detect("labas vakaras", None);
        assert!(!first.from_cache);
        assert_eq!(d.stats.misses, 1);
        let second = d.detect("labas vakaras", None);
        assert!(second.from_cache);
        assert_eq!(d.stats.hits, 1);
        assert_eq!(first.segments, second.segments);
        // Different declared language = different cache key.
        let third = d.detect("labas vakaras", Some(Lang::En));
        assert!(!third.from_cache);
    }

    #[test]
    fn cap_zero_forces_fallback_on_all_segments() {
        // cap 0ns: everything routes to CapFallback with the declared answer.
        let d = Detector::new(DetectOptions {
            cap: Duration::ZERO,
            ..DetectOptions::default()
        });
        let mut d = d;
        let u = d.detect("hello world. labas vakaras", Some(Lang::En));
        assert!(u.segments.iter().all(|s| {
            s.decision.layer == Layer::CapFallback && s.decision.over_cap && s.decision.lang == Lang::En
        }));
    }

    #[test]
    fn oversize_input_is_truncated_not_panicked() {
        let mut d = det();
        let big = "labas vakaras ".repeat(2000); // ~28KB > 16KiB
        let u = d.detect(&big, None);
        assert!(u.truncated);
        assert!(!u.segments.is_empty());
        // And a multi-byte char right at the boundary must not panic.
        let evil = format!("{}ąąąąąąąąąą", "x".repeat(16 * 1024 + 3));
        let u = d.detect(&evil, None);
        assert!(u.truncated);
    }

    #[test]
    fn empty_input_keeps_shape() {
        let u = det().detect("", None);
        assert_eq!(u.segments.len(), 1);
        assert_eq!(u.segments[0].decision.lang, Lang::Lt);
    }

    #[test]
    fn big_input_stays_under_default_cap() {
        // 64KB of real text through the FULL ladder (cache bypassed once).
        let text = "labai ilgas sakinys apie orą ir vėją. ".repeat(1600);
        let mut d = Detector::new(DetectOptions {
            max_bytes: 70 * 1024, // above the 64KB input; cap still enforced
            ..DetectOptions::default()
        });
        let u = d.detect(&text, None);
        assert!(!u.truncated);
        assert!(u.segments.iter().all(|s| !s.decision.over_cap || s.decision.layer == Layer::CapFallback));
    }
}
