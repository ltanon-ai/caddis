//! akis_json.rs — CARD-0271. Minimal std-only JSON parser for the
//! AKIS LSP lane. Hand-rolled: no deps, no tokio. Parses the subset
//! rust-analyzer / publishDiagnostics emits (objects, arrays, strings,
//! numbers, bools, null) and offers nested field access.

/// A parsed JSON value. Objects keep insertion order (Vec, not a
/// HashMap) so the lane stays std-only and field lookup is linear over
/// the small object sizes LSP messages produce.
#[derive(Debug, Clone)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    /// Nested field lookup: `self.get("params")?.get("diagnostics")`.
    pub fn get(&self, key: &str) -> Option<&Json> {
        if let Json::Obj(m) = self {
            m.iter().find(|(k, _)| k == key).map(|(_, v)| v)
        } else {
            None
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        if let Json::Str(s) = self {
            Some(s)
        } else {
            None
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        if let Json::Num(n) = self {
            Some(*n as i64)
        } else {
            None
        }
    }

    pub fn as_array(&self) -> Option<&Vec<Json>> {
        if let Json::Arr(a) = self {
            Some(a)
        } else {
            None
        }
    }

    /// Render a scalar (code may be string or number) to a string.
    pub fn display(&self) -> String {
        match self {
            Json::Str(s) => s.clone(),
            Json::Num(n) => n.to_string(),
            Json::Bool(b) => b.to_string(),
            Json::Null => "null".into(),
            _ => String::new(),
        }
    }
}

/// Parse a JSON document. `None` on any malformation (the lane is
/// advisory — a bad frame is skipped, never a gate).
pub fn parse(input: &[u8]) -> Option<Json> {
    let text = std::str::from_utf8(input).ok()?;
    let mut p = Parser {
        s: text.as_bytes(),
        i: 0,
    };
    p.ws();
    p.value()
}

struct Parser<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Parser<'a> {
    fn ws(&mut self) {
        while self.i < self.s.len() && matches!(self.s[self.i], b' ' | b'\t' | b'\n' | b'\r') {
            self.i += 1;
        }
    }

    fn value(&mut self) -> Option<Json> {
        self.ws();
        match *self.s.get(self.i)? {
            b'{' => self.object(),
            b'[' => self.array(),
            b'"' => self.string().map(Json::Str),
            b't' => self.lit("true", Json::Bool(true)),
            b'f' => self.lit("false", Json::Bool(false)),
            b'n' => self.lit("null", Json::Null),
            _ => self.number(),
        }
    }

    fn object(&mut self) -> Option<Json> {
        self.i += 1; // {
        let mut m = Vec::new();
        self.ws();
        if self.s.get(self.i) == Some(&b'}') {
            self.i += 1;
            return Some(Json::Obj(m));
        }
        loop {
            self.ws();
            let k = self.string()?;
            self.ws();
            if self.s.get(self.i) != Some(&b':') {
                return None;
            }
            self.i += 1;
            let v = self.value()?;
            m.push((k, v));
            self.ws();
            match self.s.get(self.i) {
                Some(&b',') => self.i += 1,
                Some(&b'}') => {
                    self.i += 1;
                    return Some(Json::Obj(m));
                }
                _ => return None,
            }
        }
    }

    fn array(&mut self) -> Option<Json> {
        self.i += 1; // [
        let mut a = Vec::new();
        self.ws();
        if self.s.get(self.i) == Some(&b']') {
            self.i += 1;
            return Some(Json::Arr(a));
        }
        loop {
            let v = self.value()?;
            a.push(v);
            self.ws();
            match self.s.get(self.i) {
                Some(&b',') => self.i += 1,
                Some(&b']') => {
                    self.i += 1;
                    return Some(Json::Arr(a));
                }
                _ => return None,
            }
        }
    }

    fn string(&mut self) -> Option<String> {
        if self.s.get(self.i) != Some(&b'"') {
            return None;
        }
        self.i += 1;
        let mut buf: Vec<u8> = Vec::new();
        while self.i < self.s.len() {
            let c = self.s[self.i];
            self.i += 1;
            match c {
                b'"' => return String::from_utf8(buf).ok(),
                b'\\' => {
                    let e = *self.s.get(self.i)?;
                    self.i += 1;
                    match e {
                        b'"' => buf.push(b'"'),
                        b'\\' => buf.push(b'\\'),
                        b'/' => buf.push(b'/'),
                        b'n' => buf.push(b'\n'),
                        b't' => buf.push(b'\t'),
                        b'r' => buf.push(b'\r'),
                        b'b' => buf.extend(&[0x08]),
                        b'f' => buf.extend(&[0x0c]),
                        b'u' => {
                            let hex = std::str::from_utf8(self.s.get(self.i..self.i + 4)?).ok()?;
                            let cp = u32::from_str_radix(hex, 16).ok()?;
                            self.i += 4;
                            if let Some(ch) = char::from_u32(cp) {
                                buf.extend(ch.encode_utf8(&mut [0u8; 4]).as_bytes());
                            }
                        }
                        _ => return None,
                    }
                }
                _ => buf.push(c), // raw byte: UTF-8 preserved, decoded at close
            }
        }
        None
    }

    fn number(&mut self) -> Option<Json> {
        let start = self.i;
        while self.i < self.s.len()
            && matches!(
                self.s[self.i],
                b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E'
            )
        {
            self.i += 1;
        }
        let t = std::str::from_utf8(self.s.get(start..self.i)?).ok()?;
        t.parse::<f64>().ok().map(Json::Num)
    }

    fn lit(&mut self, word: &str, v: Json) -> Option<Json> {
        if self.s.get(self.i..self.i + word.len()) == Some(word.as_bytes()) {
            self.i += word.len();
            Some(v)
        } else {
            None
        }
    }
}
