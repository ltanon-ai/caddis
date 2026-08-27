//! multipart.rs — minimal multipart/form-data, parse and build (std only).
//!
//! The organ receives multipart from real browsers (mic.html) and pipes
//! multipart to whisper-server's `/inference`. Both directions are hand-rolled
//! here because the crate is zero-dependency by law, and because the SURFACE
//! is deliberately tiny: `name="file"` (audio) plus a handful of text fields.
//! This is not a general MIME implementation and must never grow into one —
//! unknown parts are preserved as-is on parse (whisper may add fields), and
//! the builder emits exactly the shapes the two endpoints need.
//!
//! Bounds: parsing runs on a body ALREADY capped by the guards (64 MiB), so
//! the parser refuses anything larger as a defense in depth, and never
//! allocates proportional to attacker-chosen counts: output size is bounded
//! by input size, period.

/// Hard ceiling on what the parser will even look at (guards cap first).
pub const PARSE_CAP: usize = 64 * 1024 * 1024 + 1024;

/// One parsed part.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Part {
    pub name: String,
    /// Client-declared filename, when the part carries one. DISCARDED by the
    /// transcribe path (never trusted, never written to disk as a name).
    pub filename: Option<String>,
    /// The part's bytes. For text fields this is the raw value.
    pub data: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MultipartErr(pub &'static str);

impl std::fmt::Display for MultipartErr {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "multipart: {}", self.0)
    }
}

/// Pull `boundary=...` out of a Content-Type header value. Returns the
/// boundary WITHOUT quotes; quoted boundaries are unquoted once.
pub fn boundary_from_content_type(ctype: &str) -> Option<String> {
    let idx = ctype.find("boundary=")?;
    let raw = ctype[idx + "boundary=".len()..].trim();
    let cut = |s: &str| -> String {
        // The boundary ends at the next ';' parameter or end of line.
        let end = s.find(';').unwrap_or(s.len());
        s[..end].trim().to_string()
    };
    let b = cut(raw);
    let b = if b.len() >= 2 && b.starts_with('"') && b.ends_with('"') {
        b[1..b.len() - 1].to_string()
    } else {
        b
    };
    if b.is_empty() {
        None
    } else {
        Some(b)
    }
}

fn find(haystack: &[u8], needle: &[u8], from: usize) -> Option<usize> {
    if needle.is_empty() || haystack.len() < needle.len() {
        return None;
    }
    haystack[from..]
        .windows(needle.len())
        .position(|w| w == needle)
        .map(|p| p + from)
}

/// Parse a full multipart body. Splits on `--boundary`, reads each part's
/// Content-Disposition for name/filename, and takes everything after the
/// blank line as data. The final `--boundary--` terminator is required.
pub fn parse(body: &[u8], boundary: &str) -> Result<Vec<Part>, MultipartErr> {
    if body.len() > PARSE_CAP {
        return Err(MultipartErr("body over parse cap"));
    }
    let delim = format!("--{boundary}");
    let delim_b = delim.as_bytes();
    let mut parts = Vec::new();
    let mut pos = match find(body, delim_b, 0) {
        Some(p) => p + delim_b.len(),
        None => return Err(MultipartErr("no opening boundary")),
    };
    loop {
        // After a delimiter: `--` means the end; CRLF starts the next part.
        if body[pos..].starts_with(b"--") {
            break;
        }
        if !body[pos..].starts_with(b"\r\n") {
            return Err(MultipartErr("malformed boundary delimiter"));
        }
        let head_start = pos + 2;
        let head_end = match find(body, b"\r\n\r\n", head_start) {
            Some(p) => p,
            None => return Err(MultipartErr("part has no header/body split")),
        };
        let head = std::str::from_utf8(&body[head_start..head_end])
            .map_err(|_| MultipartErr("part headers not UTF-8"))?;
        let data_start = head_end + 4;
        let next_delim =
            find(body, delim_b, data_start).ok_or(MultipartErr("unterminated part"))?;
        // Data runs to the CRLF immediately before the next delimiter.
        let mut data_end = next_delim;
        if data_end >= 2 && &body[data_end - 2..data_end] == b"\r\n" {
            data_end -= 2;
        }
        let (name, filename) = parse_disposition(head);
        parts.push(Part {
            name,
            filename,
            data: body[data_start..data_end].to_vec(),
        });
        pos = next_delim + delim_b.len();
    }
    Ok(parts)
}

/// Extract `name` (required) and `filename` (optional) from a part's
/// Content-Disposition header line. Returns ("", None) for header blocks
/// without a usable disposition — the caller decides whether that is fatal.
fn parse_disposition(head: &str) -> (String, Option<String>) {
    let mut name = String::new();
    let mut filename = None;
    for line in head.split("\r\n") {
        let lower = line.to_ascii_lowercase();
        if !lower.starts_with("content-disposition:") {
            continue;
        }
        for attr in line.split(';').skip(1) {
            let attr = attr.trim();
            if let Some(v) = attr.strip_prefix("name=") {
                name = unquote(v);
            } else if let Some(v) = attr.strip_prefix("filename=") {
                filename = Some(unquote(v));
            }
        }
    }
    (name, filename)
}

fn unquote(s: &str) -> String {
    let s = s.trim();
    if s.len() >= 2 && s.starts_with('"') && s.ends_with('"') {
        s[1..s.len() - 1].to_string()
    } else {
        s.to_string()
    }
}

/// Look up the first part with this name.
pub fn part<'a>(parts: &'a [Part], name: &str) -> Option<&'a Part> {
    parts.iter().find(|p| p.name == name)
}

/// Decode a text field as UTF-8 (lossy: fields are hints, not contracts).
pub fn field(parts: &[Part], name: &str) -> Option<String> {
    part(parts, name).map(|p| String::from_utf8_lossy(&p.data).to_string())
}

// ---------------------------------------------------------------------------
// Builder — the exact shapes whisper-server's /inference accepts (proven by
// stt-daemon/stt_gpu.py, unchanged in production for a year of dictation).
// ---------------------------------------------------------------------------

/// Build a multipart body carrying one audio `file` part (audio/wav) plus
/// ordered text fields. Returns (body, content_type), or an error when the
/// audio contains every candidate boundary — a collision would make the body
/// ambiguous, and a REFUSAL is always cheaper than a corrupted transcription.
pub fn build(
    file_bytes: &[u8],
    fields: &[(&str, &str)],
) -> Result<(Vec<u8>, String), MultipartErr> {
    // Deterministic candidates, no RNG needed: salt the prefix until one is
    // absent from the audio.
    let mut boundary = String::from("----caddisvoicehorn0123456789");
    let mut salt: u64 = 0;
    while find(file_bytes, boundary.as_bytes(), 0).is_some() {
        salt += 1;
        if salt > 64 {
            return Err(MultipartErr("audio contains every boundary candidate"));
        }
        boundary = format!("----caddisvoicehorn{salt}0123456789");
    }

    let mut out = Vec::with_capacity(file_bytes.len() + 256);
    let push = |s: &str, out: &mut Vec<u8>| out.extend_from_slice(s.as_bytes());
    push(
        &format!(
            "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.wav\"\r\nContent-Type: audio/wav\r\n\r\n"
        ),
        &mut out,
    );
    out.extend_from_slice(file_bytes);
    push("\r\n", &mut out);
    for (k, v) in fields {
        push(
            &format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n"),
            &mut out,
        );
    }
    push(&format!("--{boundary}--\r\n"), &mut out);
    Ok((out, format!("multipart/form-data; boundary={boundary}")))
}

/// Build a multipart body of TEXT FIELDS ONLY (the `path`-source requests:
/// no file part at all — a present-but-empty file part is a 400 by contract,
/// not a fallback trigger).
pub fn build_fields(fields: &[(&str, &str)]) -> Result<(Vec<u8>, String), MultipartErr> {
    let boundary = String::from("----caddisvoicehornfields0123");
    let mut out = Vec::with_capacity(256);
    for (k, v) in fields {
        out.extend_from_slice(
            format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{k}\"\r\n\r\n{v}\r\n")
                .as_bytes(),
        );
    }
    out.extend_from_slice(format!("--{boundary}--\r\n").as_bytes());
    Ok((out, format!("multipart/form-data; boundary={boundary}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn boundary_extraction_handles_plain_quoted_and_trailing_params() {
        assert_eq!(
            boundary_from_content_type("multipart/form-data; boundary=XYZ"),
            Some("XYZ".into())
        );
        assert_eq!(
            boundary_from_content_type("multipart/form-data; boundary=\"a b c\""),
            Some("a b c".into())
        );
        assert_eq!(
            boundary_from_content_type("multipart/form-data; boundary=abc; charset=utf-8"),
            Some("abc".into())
        );
        assert_eq!(boundary_from_content_type("application/json"), None);
    }

    #[test]
    fn roundtrip_file_plus_fields() {
        let wav: &[u8] = &[0x52, 0x49, 0x46, 0x46, 1, 2, 3, 4, 0xFF, 0x00];
        let (body, ctype) = build(wav, &[("response_format", "json"), ("language", "lt")]).unwrap();
        let b = boundary_from_content_type(&ctype).unwrap();
        let parts = parse(&body, &b).unwrap();
        assert_eq!(parts.len(), 3);
        let f = part(&parts, "file").unwrap();
        assert_eq!(f.data, wav);
        assert_eq!(f.filename.as_deref(), Some("a.wav"));
        assert_eq!(field(&parts, "response_format").as_deref(), Some("json"));
        assert_eq!(field(&parts, "language").as_deref(), Some("lt"));
    }

    #[test]
    fn parse_rejects_garbage() {
        assert!(parse(b"not multipart at all", "b").is_err());
        assert!(parse(
            b"--b\r\nContent-Disposition: form-data; name=\"x\"\r\n\r\n",
            "b"
        )
        .is_err()); // unterminated
    }

    #[test]
    fn binary_data_with_crlf_and_boundary_like_bytes_survives() {
        // Data containing \r\n sequences and even "--" runs must survive;
        // only the exact full boundary can split parts.
        let tricky: Vec<u8> = vec![b'-', b'-', 0x0d, 0x0a, 0x00, 0x01, 0x0d, 0x0a, b'x'];
        let (body, ctype) = build(&tricky, &[]).unwrap();
        let b = boundary_from_content_type(&ctype).unwrap();
        let parts = parse(&body, &b).unwrap();
        assert_eq!(part(&parts, "file").unwrap().data, tricky);
    }

    #[test]
    fn parser_is_bounded() {
        let huge = vec![0u8; PARSE_CAP + 1];
        assert!(parse(&huge, "b").is_err());
    }
}
