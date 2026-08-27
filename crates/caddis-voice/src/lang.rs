//! lang.rs — the two organ languages (v1) + sentence splitting.
//!
//! v1 arsenal speaks exactly LT (Leonas/Ona, network edge-tts) and EN
//! (Piper ryan/amy, offline). The amendment (T-35) killed the LT→EN notice
//! swap: LT text MUST reach an LT voice. Everything downstream keys on
//! [`Lang`].

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Lang {
    Lt,
    En,
}

impl Lang {
    pub fn as_str(self) -> &'static str {
        match self {
            Lang::Lt => "lt",
            Lang::En => "en",
        }
    }

}

impl std::str::FromStr for Lang {
    type Err = &'static str;
    fn from_str(s: &str) -> Result<Lang, Self::Err> {
        match s.trim().to_ascii_lowercase().as_str() {
            "lt" | "lit" | "lithuanian" => Ok(Lang::Lt),
            "en" | "eng" | "english" => Ok(Lang::En),
            _ => Err("expected lt|en"),
        }
    }
}

/// Split an utterance into sentence-like segments on hard boundaries:
/// `. ! ? ; : \n`. Soft separators (commas) do NOT split — a clause is not a
/// language decision (council F-A1: per-SENTENCE split for mixed utterances;
/// single-language utterances stay whole). Leading/trailing whitespace of
/// each segment is trimmed; empty segments are dropped.
pub fn split_sentences(text: &str) -> Vec<&str> {
    text.split(['.', '!', '?', ';', ':', '\n'])
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_on_hard_boundaries_only() {
        let segs = split_sentences("Sveiki, pone. Kaip sekasi? Gerai! Viskas tvarkoje.");
        assert_eq!(segs, vec!["Sveiki, pone", "Kaip sekasi", "Gerai", "Viskas tvarkoje"]);
    }

    #[test]
    fn comma_does_not_split() {
        assert_eq!(split_sentences("hello, world, and goodbye"), vec!["hello, world, and goodbye"]);
    }

    #[test]
    fn newline_and_colon_split_and_empties_drop() {
        assert_eq!(split_sentences("a\n\nb. . :c"), vec!["a", "b", "c"]);
    }

    #[test]
    fn lang_roundtrip() {
        for l in [Lang::Lt, Lang::En] {
            assert_eq!(l.as_str().parse::<Lang>(), Ok(l));
        }
        assert_eq!(" EN ".parse::<Lang>(), Ok(Lang::En));
        assert_eq!("xx".parse::<Lang>(), Err("expected lt|en"));
    }
}
