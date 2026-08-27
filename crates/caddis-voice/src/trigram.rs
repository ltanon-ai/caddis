//! trigram.rs — the L2 offline language classifier (council F-A1, T-35).
//!
//! Deterministic character-trigram Naive Bayes over two embedded, parallel,
//! self-written corpora (same semantic content, LT vs EN orthotactics).
//! NO LLM, NO network, NO dictionary lookup — detection never leaves the
//! process (router precedent: the prompt-injection surface stays out of the
//! control path). Input is normalized: lowercase, Lithuanian diacritics
//! folded to base letters (L2 usually sees UNMARKED text — L1 already caught
//! the marked kind — but partial marking must not blind it), everything
//! non-alphabetic collapses to a single word boundary.
//!
//! Ambiguity is a first-class outcome: when the log-odds margin is under
//! [`AMBIGUOUS_DIFF_NATS`] the verdict carries `ambiguous: true` and the
//! DETECTION ladder applies the F-A1 tie-break (uncertain LT/EN → route LT:
//! Leonas reading EN is accented-but-correct; ryan reading LT is the banned
//! garbage).

use crate::lang::Lang;
use std::collections::HashMap;
use std::sync::LazyLock;

/// Log-odds margin (in nats) below which a verdict is ambiguous.
/// ~1 nat ≈ e:1 odds. Below that the tie-break owns the decision.
const AMBIGUOUS_DIFF_NATS: f64 = 1.0;

/// Fewer normalized letters than this → nothing to classify (None).
const MIN_LETTERS: usize = 3;

/// Parallel LT corpus (self-written; diacritics fold to base letters).
const LT_CORPUS: &str = "labas vakaras mano mielas drauge kaip tau šiandien sekasi \
    labai ačiū už tavo pagalbą ir gerus žodžius namie viskas gerai mums patinka skaityti \
    knygas ir klausytis muzikos ryte mes einame pasivaikščioti į parką oras šiandien geras \
    saulė šviečia vėjas lengvai pučia mes kalbame lietuviškai kasdien darome pratimus \
    ir mokomės naujų žodžių ši mokykla yra sena bet labai graži vaikai žaidžia kieme \
    po pamokų mano brolis turi katę ir šunį jie gyvena kaime netoli ežero ir miško \
    vakare visa šeima vakarieniauja kartu mama verda sriubą ir kepa duoną tėtis skaito \
    laikraštį o aš rašau laišką senam draugui žiemą sniegas dengia laukus ir medžius \
    pavasarį žydi obelys vasarą saulė kaitina rudenį lapai krinta ant žemės lietuva \
    yra maža šalis su ilga istorija ir gražia gamta upės teka pro laukus miestai auga \
    žmonės dirba ir švenčia šventes draugai susitinka aikštėje ir kalbasi apie orus \
    bei naujienas gatvėse eina žmonės parduotuvėse perkama duona ir pienas mokytojai \
    moko vaikus rašyti ir skaityti lietuviškai";

/// Parallel EN corpus (same content, EN orthotactics — topic cancels out).
const EN_CORPUS: &str = "good evening my dear friend how are you doing today thank you \
    very much for your help and kind words everything is fine at home we like reading \
    books and listening to music in the morning we go for a walk in the park the weather \
    is nice today the sun is shining and the wind blows gently we speak english every \
    day and learn new words this school is old but very beautiful the children play in \
    the yard after classes my brother has a cat and a dog they live in the village near \
    the lake and the forest in the evening the whole family has dinner together mother \
    makes soup and bakes bread father reads the newspaper and i write a letter to an \
    old friend in winter the snow covers the fields and the trees in spring the apple \
    trees bloom in summer the sun heats in autumn the leaves fall to the ground england \
    is a small country with a long history and beautiful nature rivers flow through \
    the fields cities grow people work and celebrate holidays friends meet in the \
    square and talk about the weather and the news people walk in the streets bread \
    and milk are bought in the shops teachers teach the children to write and read \
    in english";

/// A trigram profile: smoothed log-probabilities over ASCII trigrams.
struct Profile {
    counts: HashMap<[u8; 3], u32>,
    total: u64,
}

impl Profile {
    fn build(corpus: &str) -> Profile {
        let norm = normalize(corpus);
        let bytes = norm.as_bytes();
        let mut counts: HashMap<[u8; 3], u32> = HashMap::new();
        if bytes.len() >= 3 {
            for w in bytes.windows(3) {
                *counts.entry([w[0], w[1], w[2]]).or_insert(0) += 1;
            }
        }
        let total = counts.values().map(|&c| c as u64).sum();
        Profile { counts, total }
    }

    /// Sum of ln P(trigram) over the normalized text (add-one smoothing).
    fn score(&self, text: &str) -> f64 {
        let norm = normalize(text);
        let bytes = norm.as_bytes();
        let vocab = 27 * 27 * 27; // a-z + space, folded
        let denom = (self.total + vocab as u64) as f64;
        if bytes.len() < 3 {
            return 0.0;
        }
        bytes
            .windows(3)
            .map(|w| {
                let c = self.counts.get(&[w[0], w[1], w[2]]).copied().unwrap_or(0);
                ((c + 1) as f64 / denom).ln()
            })
            .sum()
    }
}

struct Profiles {
    lt: Profile,
    en: Profile,
}

fn profiles() -> &'static Profiles {
    static P: LazyLock<Profiles> = LazyLock::new(|| Profiles {
        lt: Profile::build(LT_CORPUS),
        en: Profile::build(EN_CORPUS),
    });
    &P
}

/// Fold one char to its normalized alphabet: a-z (LT diacritics folded) or
/// `None` for a word boundary. Uppercase folds via lowercase first.
fn fold_char(ch: char) -> Option<u8> {
    let lc = ch.to_lowercase().next().unwrap_or(ch);
    match lc {
        'a'..='z' => Some(lc as u8),
        'ą' => Some(b'a'),
        'č' => Some(b'c'),
        'ę' | 'ė' => Some(b'e'),
        'į' => Some(b'i'),
        'š' => Some(b's'),
        'ų' | 'ū' => Some(b'u'),
        'ž' => Some(b'z'),
        _ => None,
    }
}

/// Normalize: lowercase fold, diacritics → base letters, non-letters → single
/// space, leading/trailing trimmed. Output is lowercase ASCII + single spaces.
pub fn normalize(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push(' '); // boundary padding so word-edge trigrams exist
    let mut last_space = true; // the padding space already is a boundary
    for ch in text.chars() {
        match fold_char(ch) {
            Some(b) if b != b' ' => {
                out.push(b as char);
                last_space = false;
            }
            _ => {
                if !last_space {
                    out.push(' ');
                    last_space = true;
                }
            }
        }
    }
    while out.ends_with(' ') {
        out.pop();
    }
    out.push(' ');
    out
}

/// L2 verdict for one segment of text.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TrigramVerdict {
    pub lang: Lang,
    /// 0.0..1.0 — normalized log-odds margin in the winner's favor.
    pub confidence: f64,
    /// Margin under [`AMBIGUOUS_DIFF_NATS`]: the ladder must tie-break to LT.
    pub ambiguous: bool,
}

/// Classify a segment. `None` = too few letters to even try (caller falls
/// back to the declared default / LT tie-break).
pub fn classify(text: &str) -> Option<TrigramVerdict> {
    let letters = text
        .chars()
        .filter(|c| c.is_ascii_alphabetic() || "ąčęėįšųūžĄČĘĖĮŠŲŪŽ".contains(*c))
        .count();
    if letters < MIN_LETTERS {
        return None;
    }
    let p = profiles();
    let s_lt = p.lt.score(text);
    let s_en = p.en.score(text);
    let diff = s_lt - s_en; // > 0 → LT (log-space: less negative wins)
    let lang = if diff >= 0.0 { Lang::Lt } else { Lang::En };
    let margin = diff.abs();
    Some(TrigramVerdict {
        lang,
        confidence: margin / (1.0 + margin),
        ambiguous: margin < AMBIGUOUS_DIFF_NATS,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_folds_and_bounds() {
        assert_eq!(normalize("Labas, vakaras!"), " labas vakaras ");
        assert_eq!(normalize("  ĄČĘĖĮŠŲŪŽ  "), " aceeisuuz ");
    }

    #[test]
    fn marked_lt_text_classifies_lt() {
        let v = classify("labai ačiū už pagalbą").unwrap();
        assert_eq!(v.lang, Lang::Lt);
        assert!(!v.ambiguous);
    }

    #[test]
    fn unmarked_lt_classifies_lt() {
        // The whole point of L2: no diacritics anywhere.
        let v = classify("labas vakaras mano mielas drauge").unwrap();
        assert_eq!(v.lang, Lang::Lt);
        assert!(!v.ambiguous);
    }

    #[test]
    fn english_classifies_en() {
        let v = classify("good evening my dear friend how are you today").unwrap();
        assert_eq!(v.lang, Lang::En);
        assert!(!v.ambiguous);
    }

    #[test]
    fn short_english_word_is_en() {
        let v = classify("hello world").unwrap();
        assert_eq!(v.lang, Lang::En);
    }

    #[test]
    fn too_few_letters_is_none() {
        assert!(classify("123 456").is_none());
        assert!(classify("ok").is_none());
        assert!(classify("").is_none());
    }

    #[test]
    fn unseen_gibberish_is_ambiguous() {
        // Neither language: the tie-break (upstream) must own this.
        let v = classify("xyzzyx qq").unwrap();
        assert!(v.ambiguous);
    }

    #[test]
    fn confidence_is_bounded() {
        for t in ["labas vakaras", "hello world", "xyzzy qq", "viskas gerai"] {
            if let Some(v) = classify(t) {
                assert!((0.0..=1.0).contains(&v.confidence), "{t}: {}", v.confidence);
            }
        }
    }
}
