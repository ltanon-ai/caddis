//! parse.rs — turn qmd's mixed output into typed results.
//!
//! Two surfaces, both probed live 2026-08-26:
//! - `search`/`query` with `--json` emit an array of flat hit objects — but
//!   the DEEP lane (`query`) prefixes progress lines ("Warning: N documents…
//!   need embeddings", "Expanding query… (17.0s)", the `├─ lex:` tree,
//!   "Reranking 1 chunks… (3.1s)") on the SAME stream before the JSON. The
//!   parser therefore scans line starts for a `[`/`{` and takes the FIRST
//!   candidate from which a complete strict document parses (json.rs rejects
//!   trailing bytes, so a mis-picked start cannot succeed halfway).
//! - `get` ignores `--json` and always emits plain text: a `qmd://…  #docid`
//!   header, optional `Folder Context:` lines, a `---` separator, then
//!   `N: text` numbered body lines.

use crate::json::{self, Value};

#[derive(Debug, Clone, PartialEq)]
pub struct Hit {
    pub docid: String,
    pub score: Option<f64>,
    pub file: String,
    pub line: Option<u32>,
    pub title: Option<String>,
    pub context: Option<String>,
    pub snippet: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct GetDoc {
    pub file: String,
    pub docid: String,
    pub folder_context: Option<String>,
    /// Numbered body lines exactly as qmd printed them (`N` → line text).
    pub lines: Vec<(u32, String)>,
}

#[derive(Debug, Clone, PartialEq)]
pub enum ParseErr {
    NoJsonFound,
    BadJson { at: usize, msg: String },
    NotAnArray,
    MalformedHit { index: usize, why: &'static str },
    NotGetOutput(&'static str),
    BadLineNo { index: usize },
}

/// Find and parse the JSON document inside mixed stdout (progress lines +
/// JSON tail). Returns the parsed value.
pub fn json_tail(buf: &str) -> Result<Value, ParseErr> {
    let mut candidate = 0usize;
    while let Some(rel) = buf[candidate..].find(['[', '{']) {
        let at = candidate + rel;
        let starts_line = buf[..at].ends_with('\n') || at == 0;
        if starts_line {
            if let Ok(v) = json::parse(&buf[at..]) {
                return Ok(v);
            }
        }
        candidate = at + 1;
    }
    Err(ParseErr::NoJsonFound)
}

pub fn parse_hits(buf: &str) -> Result<Vec<Hit>, ParseErr> {
    let v = json_tail(buf)?;
    let arr = v.as_arr().ok_or(ParseErr::NotAnArray)?;
    let mut out = Vec::with_capacity(arr.len());
    for (index, item) in arr.iter().enumerate() {
        let obj = item.as_obj().ok_or(ParseErr::MalformedHit { index, why: "hit is not an object" })?;
        if !obj.iter().any(|(k, _)| k == "file") {
            return Err(ParseErr::MalformedHit { index, why: "hit has no file" });
        }
        let docid = item
            .get("docid")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let file = item.get("file").and_then(Value::as_str).unwrap_or("").to_string();
        if file.is_empty() {
            return Err(ParseErr::MalformedHit { index, why: "empty file" });
        }
        out.push(Hit {
            docid,
            score: item.get("score").and_then(Value::as_f64),
            file,
            line: item.get("line").and_then(Value::as_f64).map(|n| n as u32),
            title: item.get("title").and_then(Value::as_str).map(str::to_string),
            context: item.get("context").and_then(Value::as_str).map(str::to_string),
            snippet: item.get("snippet").and_then(Value::as_str).map(str::to_string),
        });
    }
    Ok(out)
}

/// Parse `qmd get` plain output. Header must be a qmd:// URI; body lines are
/// `N: text` with the number strictly increasing.
pub fn parse_get(buf: &str) -> Result<GetDoc, ParseErr> {
    let mut lines = buf.lines();
    let header = lines.next().ok_or(ParseErr::NotGetOutput("empty output"))?;
    let rest = header.strip_prefix("qmd://").ok_or(ParseErr::NotGetOutput("no qmd:// header"))?;
    // header shape: "<file>  #<docid>" (two spaces before the hash observed live)
    let (file, docid) = match rest.split_once("  ") {
        Some((f, d)) => (f.to_string(), d.trim().to_string()),
        None => (rest.trim().to_string(), String::new()),
    };
    if file.is_empty() {
        return Err(ParseErr::NotGetOutput("empty file in header"));
    }

    let mut folder_context: Option<String> = None;
    let mut body: Vec<(u32, String)> = Vec::new();
    for raw in lines {
        if let Some(ctx) = raw.strip_prefix("Folder Context: ") {
            folder_context = Some(ctx.to_string());
            continue;
        }
        if raw.trim() == "---" {
            continue;
        }
        if raw.is_empty() {
            continue;
        }
        let (num, text) = raw
            .split_once(": ")
            .ok_or(ParseErr::NotGetOutput("body line is not `N: text`"))?;
        let n: u32 = num
            .parse()
            .map_err(|_| ParseErr::NotGetOutput("body line has no number"))?;
        body.push((n, text.to_string()));
    }
    if body.is_empty() {
        return Err(ParseErr::NotGetOutput("no numbered body lines"));
    }
    // Guard against a mis-picked plain-format search block: numbers must be
    // strictly increasing line numbers.
    for i in 1..body.len() {
        if body[i].0 <= body[i - 1].0 {
            return Err(ParseErr::BadLineNo { index: i });
        }
    }
    Ok(GetDoc { file: format!("qmd://{file}"), docid, folder_context, lines: body })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEARCH_JSON: &str = r##"[
  {
    "docid": "#551652",
    "score": 0.95,
    "file": "qmd://memory/feedback-memory-md-frozen-index.md",
    "line": 2,
    "title": "feedback_memory_md_frozen_index",
    "context": "Auto-memory corpus pointer",
    "snippet": "@@ -1,4 @@ (0 before, 35 after)\n---\nname: feedback-memory-md-frozen-index"
  }
]"##;

    #[test]
    fn parses_clean_search_output() {
        let hits = parse_hits(SEARCH_JSON).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].docid, "#551652");
        assert_eq!(hits[0].score, Some(0.95));
        assert_eq!(hits[0].file, "qmd://memory/feedback-memory-md-frozen-index.md");
        assert_eq!(hits[0].line, Some(2));
    }

    #[test]
    fn parses_deep_lane_progress_prefixed_output() {
        // Shape captured live 2026-08-26: warnings, expansion tree, rerank
        // line, then the JSON array.
        let mixed = "Warning: 2 documents (100%) need embeddings. Run 'qmd embed' for better results.\n\
                     Expanding query... (17.0s)\n\
                     ├─ golden needle canary\n\
                     ├─ lex: canary spot\n\
                     ├─ vec: yellow needle bird\n\
                     └─ hyde: Golden needle canary is an important concept...\n\
                     Searching 6 queries...\n\
                     Reranking 1 chunks... (3.1s)\n"
            .to_string()
            + SEARCH_JSON;
        let hits = parse_hits(&mixed).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].docid, "#551652");
    }

    #[test]
    fn empty_result_set_is_valid() {
        assert_eq!(parse_hits("[]").unwrap(), Vec::<Hit>::new());
    }

    #[test]
    fn no_json_fails_closed() {
        assert!(matches!(parse_hits("Warning: nothing here\nno json at all"), Err(ParseErr::NoJsonFound)));
        // a '[' mid-line must not be picked as a start
        assert!(matches!(parse_hits("progress [weird] line\nstill no json"), Err(ParseErr::NoJsonFound)));
    }

    #[test]
    fn hit_without_file_is_malformed() {
        assert!(matches!(
            parse_hits(r##"[{"docid":"#1","score":0.5}]"##),
            Err(ParseErr::MalformedHit { index: 0, .. })
        ));
    }

    const GET_PLAIN: &str = "qmd://memory/docs/golden.md  #26a8bc\n\
        Folder Context: Safety-net collection capturing markdown\n\
        ---\n\
        \n\
        1: ---\n\
        2: name: golden-needle\n\
        3: description: canary fixture doc\n";

    #[test]
    fn parses_get_output() {
        let d = parse_get(GET_PLAIN).unwrap();
        assert_eq!(d.file, "qmd://memory/docs/golden.md");
        assert_eq!(d.docid, "#26a8bc");
        assert_eq!(d.folder_context.as_deref(), Some("Safety-net collection capturing markdown"));
        assert_eq!(d.lines.len(), 3);
        assert_eq!(d.lines[1], (2, "name: golden-needle".to_string()));
    }

    #[test]
    fn get_without_docid_or_context_still_parses() {
        let minimal = "qmd://x.md\n1: hello\n2: world\n";
        let d = parse_get(minimal).unwrap();
        assert_eq!(d.file, "qmd://x.md");
        assert_eq!(d.docid, "");
        assert_eq!(d.lines.len(), 2);
    }

    #[test]
    fn get_rejects_non_increasing_lines() {
        assert!(matches!(
            parse_get("qmd://x.md\n5: a\n3: b\n"),
            Err(ParseErr::BadLineNo { index: 1 })
        ));
        assert!(matches!(parse_get("not a get output at all"), Err(ParseErr::NotGetOutput(_))));
    }
}
