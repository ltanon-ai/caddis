//! json.rs — minimal JSON value parser + serializer, scoped to what the memory
//! organ reads and writes: qmd `--json` output (arrays of flat objects) and the
//! organ-owned collection registry file.
//!
//! Why hand-rolled: caddis-core's TCB law keeps serde_json dev-only until the
//! schema freezes; this crate is the first RUNTIME JSON consumer in the
//! workspace, so the parser lives here, strict and small. Full grammar:
//! null/true/false, numbers, strings with all escapes incl. surrogate pairs,
//! arrays, objects; depth-capped at 128 (fail-closed on pathological input);
//! trailing non-whitespace after a document is an error — that strictness is
//! what lets parse.rs find the JSON tail inside mixed progress-line output.

/// Maximum nesting depth before the parser refuses (defensive; qmd output is
/// 2 levels deep, the registry 3).
const MAX_DEPTH: usize = 128;

/// Cap on any single JSON document the organ will parse (8 MiB). qmd result
/// sets are kilobytes; anything past this cap is not a recall answer.
const MAX_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

impl Value {
    pub fn get(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Obj(pairs) => pairs.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[Value]> {
        match self {
            Value::Arr(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_obj(&self) -> Option<&[(String, Value)]> {
        match self {
            Value::Obj(o) => Some(o),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct JsonErr {
    pub at: usize,
    pub msg: String,
}

impl JsonErr {
    fn new(at: usize, msg: &str) -> Self {
        JsonErr { at, msg: msg.to_string() }
    }
}

/// Parse one complete JSON document. Trailing non-whitespace fails.
pub fn parse(buf: &str) -> Result<Value, JsonErr> {
    if buf.len() > MAX_BYTES {
        return Err(JsonErr::new(0, "document exceeds 8 MiB cap"));
    }
    let b = buf.as_bytes();
    let mut p = Parser { b, i: 0 };
    p.ws();
    let v = p.value(0)?;
    p.ws();
    if p.i != b.len() {
        return Err(JsonErr::new(p.i, "trailing bytes after document"));
    }
    Ok(v)
}

struct Parser<'a> {
    b: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, JsonErr> {
        if depth > MAX_DEPTH {
            return Err(JsonErr::new(self.i, "nesting deeper than 128"));
        }
        match self.b.get(self.i) {
            None => Err(JsonErr::new(self.i, "unexpected end of input")),
            Some(b'n') => self.lit(b"null", Value::Null),
            Some(b't') => self.lit(b"true", Value::Bool(true)),
            Some(b'f') => self.lit(b"false", Value::Bool(false)),
            Some(b'"') => Ok(Value::Str(self.string()?)),
            Some(b'[') => self.array(depth),
            Some(b'{') => self.object(depth),
            Some(c) if *c == b'-' || c.is_ascii_digit() => self.number(),
            Some(_) => Err(JsonErr::new(self.i, "unexpected byte for a value")),
        }
    }

    fn lit(&mut self, word: &[u8], v: Value) -> Result<Value, JsonErr> {
        if self.b[self.i..].starts_with(word) {
            self.i += word.len();
            Ok(v)
        } else {
            Err(JsonErr::new(self.i, "invalid literal"))
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, JsonErr> {
        self.i += 1; // '['
        let mut out = Vec::new();
        self.ws();
        if self.b.get(self.i) == Some(&b']') {
            self.i += 1;
            return Ok(Value::Arr(out));
        }
        loop {
            self.ws();
            out.push(self.value(depth + 1)?);
            self.ws();
            match self.b.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b']') => {
                    self.i += 1;
                    return Ok(Value::Arr(out));
                }
                _ => return Err(JsonErr::new(self.i, "expected ',' or ']' in array")),
            }
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, JsonErr> {
        self.i += 1; // '{'
        let mut out: Vec<(String, Value)> = Vec::new();
        self.ws();
        if self.b.get(self.i) == Some(&b'}') {
            self.i += 1;
            return Ok(Value::Obj(out));
        }
        loop {
            self.ws();
            if self.b.get(self.i) != Some(&b'"') {
                return Err(JsonErr::new(self.i, "expected string key in object"));
            }
            let key = self.string()?;
            self.ws();
            if self.b.get(self.i) != Some(&b':') {
                return Err(JsonErr::new(self.i, "expected ':' after key"));
            }
            self.i += 1;
            self.ws();
            let val = self.value(depth + 1)?;
            out.push((key, val));
            self.ws();
            match self.b.get(self.i) {
                Some(b',') => self.i += 1,
                Some(b'}') => {
                    self.i += 1;
                    return Ok(Value::Obj(out));
                }
                _ => return Err(JsonErr::new(self.i, "expected ',' or '}' in object")),
            }
        }
    }

    fn number(&mut self) -> Result<Value, JsonErr> {
        let start = self.i;
        if self.b.get(self.i) == Some(&b'-') {
            self.i += 1;
        }
        while matches!(self.b.get(self.i), Some(c) if c.is_ascii_digit()) {
            self.i += 1;
        }
        // Strict JSON: a leading zero may only stand alone ("0", "0.5"),
        // never prefix more digits ("01" is invalid).
        let int_len = self.i - start
            - usize::from(self.b.get(start) == Some(&b'-'));
        if int_len == 0 {
            return Err(JsonErr::new(start, "number has no digits"));
        }
        if int_len > 1 && self.b[start + usize::from(self.b.get(start) == Some(&b'-'))] == b'0' {
            return Err(JsonErr::new(start, "leading zero is not valid JSON"));
        }
        if self.b.get(self.i) == Some(&b'.') {
            self.i += 1;
            while matches!(self.b.get(self.i), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
            }
        }
        if matches!(self.b.get(self.i), Some(b'e') | Some(b'E')) {
            self.i += 1;
            if matches!(self.b.get(self.i), Some(b'+') | Some(b'-')) {
                self.i += 1;
            }
            while matches!(self.b.get(self.i), Some(c) if c.is_ascii_digit()) {
                self.i += 1;
            }
        }
        let text = std::str::from_utf8(&self.b[start..self.i])
            .map_err(|_| JsonErr::new(start, "number not utf8"))?;
        text.parse::<f64>()
            .map(Value::Num)
            .map_err(|_| JsonErr::new(start, "invalid number"))
    }

    fn string(&mut self) -> Result<String, JsonErr> {
        self.i += 1; // opening quote
        let mut out = String::new();
        loop {
            match self.b.get(self.i) {
                None => return Err(JsonErr::new(self.i, "unterminated string")),
                Some(b'"') => {
                    self.i += 1;
                    return Ok(out);
                }
                Some(b'\\') => {
                    self.i += 1;
                    match self.b.get(self.i) {
                        Some(b'"') => out.push('"'),
                        Some(b'\\') => out.push('\\'),
                        Some(b'/') => out.push('/'),
                        Some(b'b') => out.push('\u{0008}'),
                        Some(b'f') => out.push('\u{000C}'),
                        Some(b'n') => out.push('\n'),
                        Some(b'r') => out.push('\r'),
                        Some(b't') => out.push('\t'),
                        Some(b'u') => {
                            let hi = self.hex4()?;
                            let ch = if (0xD800..0xDC00).contains(&hi) {
                                // surrogate pair: \uXXXX\uXXXX
                                if self.b.get(self.i + 1) == Some(&b'\\')
                                    && self.b.get(self.i + 2) == Some(&b'u')
                                {
                                    self.i += 2;
                                    let lo = self.hex4()?;
                                    if !(0xDC00..0xE000).contains(&lo) {
                                        return Err(JsonErr::new(self.i, "invalid low surrogate"));
                                    }
                                    let c = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                                    char::from_u32(c)
                                        .ok_or_else(|| JsonErr::new(self.i, "bad codepoint"))?
                                } else {
                                    return Err(JsonErr::new(self.i, "lone high surrogate"));
                                }
                            } else if (0xDC00..0xE000).contains(&hi) {
                                return Err(JsonErr::new(self.i, "lone low surrogate"));
                            } else {
                                char::from_u32(hi)
                                    .ok_or_else(|| JsonErr::new(self.i, "bad codepoint"))?
                            };
                            out.push(ch);
                            continue; // hex4 already advanced past the last digit
                        }
                        _ => return Err(JsonErr::new(self.i, "invalid escape")),
                    }
                    self.i += 1;
                }
                Some(&c) if c < 0x20 => {
                    return Err(JsonErr::new(self.i, "raw control char in string"));
                }
                Some(_) => {
                    // copy one UTF-8 scalar: find its length by the lead byte
                    let len = utf8_len(self.b[self.i]);
                    let end = self.i + len;
                    if end > self.b.len() {
                        return Err(JsonErr::new(self.i, "truncated utf8"));
                    }
                    let s = std::str::from_utf8(&self.b[self.i..end])
                        .map_err(|_| JsonErr::new(self.i, "invalid utf8"))?;
                    out.push_str(s);
                    self.i = end;
                }
            }
        }
    }

    fn hex4(&mut self) -> Result<u32, JsonErr> {
        self.i += 1; // past 'u'
        if self.i + 4 > self.b.len() {
            return Err(JsonErr::new(self.i, "short \\u escape"));
        }
        let mut v: u32 = 0;
        for _ in 0..4 {
            let c = self.b[self.i];
            let d = (c as char)
                .to_digit(16)
                .ok_or_else(|| JsonErr::new(self.i, "non-hex in \\u escape"))?;
            v = v * 16 + d;
            self.i += 1;
        }
        Ok(v)
    }
}

fn utf8_len(b: u8) -> usize {
    if b < 0x80 {
        1
    } else if b >> 5 == 0b110 {
        2
    } else if b >> 4 == 0b1110 {
        3
    } else {
        4
    }
}

/// Serialize a value back to compact JSON (registry writes; diagnostics).
pub fn to_string(v: &Value) -> String {
    let mut out = String::new();
    write_value(v, &mut out);
    out
}

fn write_value(v: &Value, out: &mut String) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(n) => {
            if n.fract() == 0.0 && n.abs() < 1e15 {
                out.push_str(&format!("{}", *n as i64));
            } else {
                out.push_str(&format!("{n}"));
            }
        }
        Value::Str(s) => write_str(s, out),
        Value::Arr(a) => {
            out.push('[');
            for (i, item) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(item, out);
            }
            out.push(']');
        }
        Value::Obj(o) => {
            out.push('{');
            for (i, (k, val)) in o.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_str(k, out);
                out.push(':');
                write_value(val, out);
            }
            out.push('}');
        }
    }
}

fn write_str(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{0008}' => out.push_str("\\b"),
            '\u{000C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_qmd_shaped_output() {
        let raw = r##"[{"docid":"#551652","score":0.95,"file":"qmd://memory/a.md","line":2,"title":"t","context":"c","snippet":"@@ -1,4 @@\n---"}]"##;
        let v = parse(raw).unwrap();
        let arr = v.as_arr().unwrap();
        assert_eq!(arr.len(), 1);
        let hit = &arr[0];
        assert_eq!(hit.get("docid").unwrap().as_str(), Some("#551652"));
        assert_eq!(hit.get("score").unwrap().as_f64(), Some(0.95));
        assert_eq!(hit.get("line").unwrap().as_f64(), Some(2.0));
        assert_eq!(hit.get("snippet").unwrap().as_str().unwrap(), "@@ -1,4 @@\n---");
    }

    #[test]
    fn trailing_bytes_rejected() {
        assert!(parse("[1,2] junk").is_err());
        assert!(parse("  {\"a\":1}  \n").is_ok());
    }

    #[test]
    fn all_escapes_roundtrip() {
        let raw = r#""a\"b\\c\/d\bef\ng\rh\tiA😀""#;
        let v = parse(raw).unwrap();
        assert_eq!(v.as_str(), Some("a\"b\\c/d\u{0008}ef\ng\rh\tiA\u{1F600}"));
        // serialize back and re-parse (round trip through our own writer)
        let again = parse(&to_string(&v)).unwrap();
        assert_eq!(v, again);
    }

    #[test]
    fn rejects_lone_surrogates_and_bad_literals() {
        assert!(parse(r#""\uD800""#).is_err());
        assert!(parse(r#""\uD800A""#).is_err());
        assert!(parse("tru").is_err());
        assert!(parse("[1,]").is_err());
        assert!(parse("{a:1}").is_err());
    }

    #[test]
    fn numbers_full_grammar() {
        for t in ["0", "-1", "3.14", "-2.5e-3", "1E+9", "123456789"] {
            assert!(parse(t).is_ok(), "failed: {t}");
        }
        assert!(parse("01").is_err(), "leading zero is not valid JSON");
        assert_eq!(parse("-2.5e-3").unwrap().as_f64(), Some(-0.0025));
    }

    #[test]
    fn depth_cap_fails_closed() {
        let deep = "[".repeat(200) + &"]".repeat(200);
        assert!(parse(&deep).is_err());
        let ok_depth = "[".repeat(100) + &"]".repeat(100);
        assert!(parse(&ok_depth).is_ok());
    }

    #[test]
    fn writer_objects_and_order() {
        let v = Value::Obj(vec![(
            "collections".into(),
            Value::Obj(vec![(
                "memory".into(),
                Value::Obj(vec![
                    ("public".into(), Value::Bool(false)),
                    ("owner".into(), Value::Str("caddis-memory".into())),
                ]),
            )]),
        )]);
        let s = to_string(&v);
        assert_eq!(
            s,
            r#"{"collections":{"memory":{"public":false,"owner":"caddis-memory"}}}"#
        );
        assert_eq!(parse(&s).unwrap(), v);
    }
}
